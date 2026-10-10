//! Live broadcasts. The audio is fetched as fragments and played as one continuous
//! fragmented MP4, so the decoder reads it front to back and never seeks.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Fragments that may wait for the decoder. Bounds memory and the lag behind
/// the live edge, which is about three fragments of roughly five seconds each.
const QUEUED_FRAGMENTS: usize = 3;
/// A fragment arrives about this long after the previous one. Polling a little
/// before that point keeps requests close to one per fragment.
const NEXT_FRAGMENT_WAIT: Duration = Duration::from_millis(4000);
/// Polls in between fragments, when the edge has not moved yet.
const EDGE_POLL_WAIT: Duration = Duration::from_millis(1000);
/// Failed polls in a row before the broadcast is treated as ended.
const MAX_FAILED_POLLS: u32 = 5;
/// Without a new fragment for this long, the broadcast has stalled: fragments come every few seconds.
const EDGE_STALL_LIMIT: Duration = Duration::from_secs(60);
/// Fragments that playback may fall behind the head before it jumps to the head. Playback normally
/// stays within a fragment or two of it. A longer lag comes from a stall or a pause, and old audio
/// played at real-time speed would keep the lag instead of closing it.
const MAX_LAG_FRAGMENTS: u64 = 6;
/// Response headers with the fragment's sequence number and with the head's sequence number.
const SEQUENCE_HEADER: &str = "x-sequence-num";
const HEAD_SEQUENCE_HEADER: &str = "x-head-seqnum";

/// Whether the live edge has stayed on the same fragment for too long.
fn edge_stalled(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) >= EDGE_STALL_LIMIT
}
/// Pause before retrying a full queue.
const QUEUE_RETRY: Duration = Duration::from_millis(100);

/// The media the decoder reads: the first fragment's `ftyp` and `moov`, then
/// every `moof` and `mdat` in order.
pub struct LiveAudioReader {
    fragments: Receiver<Vec<u8>>,
    current: Vec<u8>,
    position: usize,
}

impl Read for LiveAudioReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        while self.position >= self.current.len() {
            match self.fragments.recv() {
                Ok(fragment) => {
                    self.current = fragment;
                    self.position = 0;
                }
                // The fetcher has stopped: the broadcast ended or the playback was cancelled.
                Err(_) => return Ok(0),
            }
        }
        let count = buf.len().min(self.current.len() - self.position);
        buf[..count].copy_from_slice(&self.current[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

impl Seek for LiveAudioReader {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a live broadcast cannot be seeked",
        ))
    }
}

/// One top-level MP4 box: its four-character type and its whole bytes, header included.
struct Mp4Box<'a> {
    kind: [u8; 4],
    bytes: &'a [u8],
}

fn top_level_boxes(data: &[u8]) -> Vec<Mp4Box<'_>> {
    let mut boxes = Vec::new();
    let mut offset = 0;
    while offset + 8 <= data.len() {
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let (size, header) = match size {
            // A 64-bit size follows the type. A size beyond this platform's range is out of bounds.
            1 if offset + 16 <= data.len() => {
                let mut large = [0; 8];
                large.copy_from_slice(&data[offset + 8..offset + 16]);
                (
                    usize::try_from(u64::from_be_bytes(large)).unwrap_or(usize::MAX),
                    16,
                )
            }
            // A zero size runs to the end of the buffer.
            0 => (data.len() - offset, 8),
            size => (size, 8),
        };
        // `offset` is below `data.len()`, so the remaining length is computed without underflow.
        // Adding the size to the offset instead could overflow on a hostile size.
        if size < header || size > data.len() - offset {
            break;
        }
        let mut kind = [0; 4];
        kind.copy_from_slice(&data[offset + 4..offset + 8]);
        boxes.push(Mp4Box {
            kind,
            bytes: &data[offset..offset + size],
        });
        offset += size;
    }
    boxes
}

/// The payload of the first child box of `kind` inside `parent` (a container box).
fn child_payload<'a>(parent: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let body = parent.get(8..)?;
    top_level_boxes(body)
        .into_iter()
        .find(|child| &child.kind == kind)
        .and_then(|child| child.bytes.get(8..))
}

/// The base media decode time of a `moof`: where its first fragment starts on the timeline.
fn fragment_start(moof: &[u8]) -> Option<u64> {
    let traf = child_payload(moof, b"traf")?;
    let tfdt = child_payload_of_box(traf, b"tfdt")?;
    let version = *tfdt.first()?;
    if version == 1 {
        let mut time = [0; 8];
        time.copy_from_slice(tfdt.get(4..12)?);
        Some(u64::from_be_bytes(time))
    } else {
        let mut time = [0; 4];
        time.copy_from_slice(tfdt.get(4..8)?);
        Some(u64::from(u32::from_be_bytes(time)))
    }
}

/// Like `child_payload`, for a `traf` body that starts with its child boxes.
fn child_payload_of_box<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    top_level_boxes(body)
        .into_iter()
        .find(|child| &child.kind == kind)
        .and_then(|child| child.bytes.get(8..))
}

/// One poll's answer, split into what the reader needs.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Fragment {
    /// Where the fragment starts on the broadcast's timeline.
    pub(super) start: u64,
    /// `ftyp` and `moov`, which describe the track. Only the first fragment carries them.
    pub(super) init: Vec<u8>,
    /// Every `moof` and `mdat`, in order.
    pub(super) media: Vec<u8>,
}

/// Splits one poll's response into the init boxes and the media boxes. Other boxes
/// (`emsg`, `sidx`, `styp`) are not needed by the decoder and are dropped.
pub(super) fn parse_fragment(data: &[u8]) -> Option<Fragment> {
    let mut start = None;
    let mut init = Vec::new();
    let mut media = Vec::new();
    for item in top_level_boxes(data) {
        match &item.kind {
            b"ftyp" | b"moov" => init.extend_from_slice(item.bytes),
            b"moof" => {
                if start.is_none() {
                    start = fragment_start(item.bytes);
                }
                media.extend_from_slice(item.bytes);
            }
            b"mdat" => media.extend_from_slice(item.bytes),
            _ => {}
        }
    }
    Some(Fragment {
        start: start?,
        init,
        media,
    })
}

/// Gives every `moof` in `media` the next sequence number. Each poll numbers its
/// fragment from 1, and the decoder expects the numbers to count up.
fn number_moofs(media: &mut [u8], next: &mut u32) {
    let be32 = |bytes: &[u8]| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let mut offset = 0;
    while offset + 8 <= media.len() {
        let size = be32(&media[offset..]);
        if size < 8 || size > media.len() - offset {
            break;
        }
        if &media[offset + 4..offset + 8] == b"moof" {
            let end = offset + size;
            let mut child = offset + 8;
            while child + 8 <= end {
                let child_size = be32(&media[child..]);
                if child_size < 8 || child_size > end - child {
                    break;
                }
                // mfhd: a full box, so the sequence number follows its 4-byte version and flags.
                if &media[child + 4..child + 8] == b"mfhd" && child_size >= 16 {
                    media[child + 12..child + 16].copy_from_slice(&next.to_be_bytes());
                    *next = next.wrapping_add(1);
                    break;
                }
                child += child_size;
            }
        }
        offset += size;
    }
}

/// The sequence number to request next. Before any answer the live edge is asked, which is the
/// newest fragment. Playback that has fallen more than `MAX_LAG_FRAGMENTS` behind the head jumps
/// to the head.
fn request_for(next: Option<u64>, head: Option<u64>) -> Option<u64> {
    let next = next?;
    match head {
        Some(head) if head.saturating_sub(next) > MAX_LAG_FRAGMENTS => Some(head),
        _ => Some(next),
    }
}

/// What an answer means for the request it answers.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// The fragment that was asked for.
    Accept,
    /// A later fragment than the one asked for. The fragments in between are gone; playback goes on from this one.
    Gap,
    /// An earlier fragment than the one asked for. It is not queued again.
    Behind,
}

fn verdict(requested: u64, sequence: u64) -> Verdict {
    match sequence.cmp(&requested) {
        std::cmp::Ordering::Equal => Verdict::Accept,
        std::cmp::Ordering::Greater => Verdict::Gap,
        std::cmp::Ordering::Less => Verdict::Behind,
    }
}

/// The fragment URL with an `sq` parameter for `sequence`. The other parameters keep their text:
/// the signature covers their values, so they are not decoded and encoded again.
fn sequenced_url(url: &reqwest::Url, sequence: u64) -> Option<reqwest::Url> {
    let (path, query) = url.as_str().split_once('?').unwrap_or((url.as_str(), ""));
    let sequence = format!("sq={sequence}");
    let parameters = query
        .split('&')
        .filter(|pair| !pair.is_empty() && pair.split('=').next() != Some("sq"))
        .chain(std::iter::once(sequence.as_str()))
        .collect::<Vec<_>>()
        .join("&");
    reqwest::Url::parse(&format!("{path}?{parameters}")).ok()
}

/// A numeric header that the broadcast server sends with its answers.
fn header_number(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

/// Tells the listener that the broadcast has stopped moving. The fetch ends after this.
fn report_stalled(fetch_error: &Arc<Mutex<Option<String>>>) {
    warn!("live edge stopped moving");
    crate::playback::engine::record_fetch_error(
        fetch_error,
        "the live broadcast stopped moving".into(),
    );
}

/// Starts fetching the broadcast's fragments. The reader it returns plays them as they arrive.
pub(super) fn open_broadcast(
    client: reqwest::Client,
    url: reqwest::Url,
    headers: reqwest::header::HeaderMap,
    cancellation: CancellationToken,
    fetch_error: Arc<Mutex<Option<String>>>,
) -> LiveAudioReader {
    let (sender, fragments) = bounded(QUEUED_FRAGMENTS);
    tokio::spawn(fetch_fragments(
        client,
        url,
        headers,
        cancellation,
        sender,
        fetch_error,
    ));
    LiveAudioReader {
        fragments,
        current: Vec::new(),
        position: 0,
    }
}

async fn fetch_fragments(
    client: reqwest::Client,
    url: reqwest::Url,
    headers: reqwest::header::HeaderMap,
    cancellation: CancellationToken,
    sender: Sender<Vec<u8>>,
    fetch_error: Arc<Mutex<Option<String>>>,
) {
    // The start time of the last fragment queued without a sequence number.
    let mut last_start = None;
    let mut sent_init = false;
    // The sequence number to fetch next. None until the first answer, and for a server that sends
    // no sequence numbers, where the live edge is polled as before.
    let mut next_sequence: Option<u64> = None;
    let mut head = None;
    // The number the decoder sees on each moof. It counts up over the whole broadcast.
    let mut next_moof = 1;
    let mut failed_polls = 0;
    // When the last new fragment was queued, or the fetch began.
    let mut last_progress = Instant::now();
    let mut wait = Duration::ZERO;
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return,
            _ = tokio::time::sleep(wait) => {}
        }
        wait = EDGE_POLL_WAIT;
        let requested = request_for(next_sequence, head);
        if let (Some(next), Some(target)) = (next_sequence, requested)
            && target != next
        {
            warn!(
                behind = target - next,
                "live playback fell behind; jumping to the live edge"
            );
        }
        let answer = match poll_once(&client, &url, &headers, &cancellation, requested).await {
            Poll::Cancelled => return,
            Poll::Fragment(answer) => {
                failed_polls = 0;
                answer
            }
            Poll::Pending(answer_head) => {
                // The fragment is not produced yet. A broadcast that stays there too long has stalled.
                head = answer_head.or(head);
                if edge_stalled(last_progress, Instant::now()) {
                    report_stalled(&fetch_error);
                    return;
                }
                continue;
            }
            Poll::Failed(message) => {
                failed_polls += 1;
                if failed_polls >= MAX_FAILED_POLLS {
                    warn!(message, "live broadcast stopped answering");
                    crate::playback::engine::record_fetch_error(&fetch_error, message);
                    return;
                }
                continue;
            }
        };
        head = answer.head.or(head);
        let fragment = answer.fragment;
        if !sent_init && fragment.init.is_empty() {
            // The reader needs the track description before any media. Poll again.
            continue;
        }
        match (requested, answer.sequence) {
            (Some(requested), Some(sequence)) => match verdict(requested, sequence) {
                Verdict::Accept => {}
                Verdict::Gap => warn!(requested, sequence, "live fragments were skipped"),
                Verdict::Behind => {
                    if edge_stalled(last_progress, Instant::now()) {
                        report_stalled(&fetch_error);
                        return;
                    }
                    continue;
                }
            },
            _ => {
                // Without sequence numbers the live edge is polled, and a repeated fragment is dropped.
                if answer.sequence.is_none() {
                    next_sequence = None;
                }
                if last_start.is_some_and(|last| fragment.start <= last) {
                    // The same fragment as the last poll: the edge has not moved. A broadcast that stays
                    // there too long has stalled, and the listener is told rather than left in silence.
                    if edge_stalled(last_progress, Instant::now()) {
                        report_stalled(&fetch_error);
                        return;
                    }
                    continue;
                }
            }
        }
        last_start = Some(fragment.start);
        if let Some(sequence) = answer.sequence {
            next_sequence = Some(sequence + 1);
        }
        let mut media = fragment.media;
        number_moofs(&mut media, &mut next_moof);
        let mut payload = if sent_init {
            media
        } else {
            let mut first = fragment.init;
            first.extend_from_slice(&media);
            first
        };
        sent_init = true;
        wait = NEXT_FRAGMENT_WAIT;
        loop {
            match sender.try_send(payload) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    payload = returned;
                    tokio::select! {
                        _ = cancellation.cancelled() => return,
                        _ = tokio::time::sleep(QUEUE_RETRY) => {}
                    }
                }
                // The reader is gone: the playback was stopped or replaced.
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        last_progress = Instant::now();
        info!(
            start = fragment.start,
            sequence = answer.sequence.unwrap_or_default(),
            "live fragment queued"
        );
    }
}

enum Poll {
    Fragment(Answer),
    /// The fragment that was asked for is not produced yet. The answer still reports the head.
    Pending(Option<u64>),
    Failed(String),
    Cancelled,
}

/// A fragment with the sequence numbers its answer carried.
struct Answer {
    fragment: Fragment,
    sequence: Option<u64>,
    head: Option<u64>,
}

/// What a refused poll reports. A 403 carries the message the engine answers with a new resolution.
fn refusal_message(status: reqwest::StatusCode) -> String {
    if status == reqwest::StatusCode::FORBIDDEN {
        crate::playback::engine::googlevideo_refusal_message()
    } else {
        format!("live poll refused: HTTP {status}")
    }
}

async fn poll_once(
    client: &reqwest::Client,
    url: &reqwest::Url,
    headers: &reqwest::header::HeaderMap,
    cancellation: &CancellationToken,
    requested: Option<u64>,
) -> Poll {
    let target = match requested {
        Some(sequence) => match sequenced_url(url, sequence) {
            Some(target) => target,
            None => return Poll::Failed("live poll URL could not be built".into()),
        },
        None => url.clone(),
    };
    let response = tokio::select! {
        _ = cancellation.cancelled() => return Poll::Cancelled,
        response = client.get(target).headers(headers.clone()).header(reqwest::header::RANGE, "bytes=0-").send() => response,
    };
    let response = match response {
        Ok(response) => response,
        // The error text of a request carries the media URL, which is signed and must not reach logs.
        Err(error) => return Poll::Failed(format!("live poll failed: {}", error.without_url())),
    };
    let status = response.status();
    let sequence = header_number(response.headers(), SEQUENCE_HEADER);
    let head = header_number(response.headers(), HEAD_SEQUENCE_HEADER);
    if status == reqwest::StatusCode::NO_CONTENT {
        // A sequence number past the head is answered with no content until the fragment exists.
        return Poll::Pending(head);
    }
    if !status.is_success() {
        return Poll::Failed(refusal_message(status));
    }
    let body = tokio::select! {
        _ = cancellation.cancelled() => return Poll::Cancelled,
        body = response.bytes() => body,
    };
    match body.ok().and_then(|bytes| parse_fragment(&bytes)) {
        Some(fragment) => Poll::Fragment(Answer {
            fragment,
            sequence,
            head,
        }),
        None => Poll::Failed("live poll returned no fragment".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box with the given type and payload, with its size header.
    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    /// A `moof` whose first traf carries a version-1 `tfdt` at `start`.
    fn moof_at(start: u64) -> Vec<u8> {
        let mut tfdt_payload = vec![1, 0, 0, 0];
        tfdt_payload.extend_from_slice(&start.to_be_bytes());
        let traf = boxed(b"traf", &boxed(b"tfdt", &tfdt_payload));
        boxed(b"moof", &traf)
    }

    fn fragment_bytes(start: u64, with_init: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if with_init {
            out.extend(boxed(b"ftyp", b"M4A "));
            out.extend(boxed(b"moov", &[0; 4]));
        }
        out.extend(boxed(b"emsg", &[0; 6]));
        out.extend(moof_at(start));
        out.extend(boxed(b"mdat", b"audio"));
        out
    }

    #[test]
    fn a_fragment_splits_into_init_media_and_its_start_time() {
        let fragment = parse_fragment(&fragment_bytes(440_000, true)).unwrap();
        assert_eq!(fragment.start, 440_000);
        assert!(fragment.init.starts_with(b"\x00\x00\x00\x0cftyp"));
        assert!(fragment.init.windows(4).any(|window| window == b"moov"));
        // Media keeps moof and mdat in order and drops the event message.
        assert!(!fragment.media.windows(4).any(|window| window == b"emsg"));
        assert!(fragment.media.starts_with(&moof_at(440_000)));
    }

    #[test]
    fn a_fragment_without_a_fragment_start_is_rejected() {
        assert_eq!(parse_fragment(&boxed(b"ftyp", b"M4A ")), None);
    }

    #[test]
    fn every_moof_gets_the_next_sequence_number() {
        let mfhd = |sequence: u32| {
            let mut payload = vec![0, 0, 0, 0];
            payload.extend_from_slice(&sequence.to_be_bytes());
            boxed(b"mfhd", &payload)
        };
        let moof = |sequence: u32| boxed(b"moof", &mfhd(sequence));
        let mut media = Vec::new();
        media.extend(moof(1));
        media.extend(boxed(b"mdat", b"one"));
        media.extend(moof(1));
        media.extend(boxed(b"mdat", b"two"));
        let mut next = 7;
        number_moofs(&mut media, &mut next);
        assert_eq!(next, 9);
        let mut expected = Vec::new();
        expected.extend(moof(7));
        expected.extend(boxed(b"mdat", b"one"));
        expected.extend(moof(8));
        expected.extend(boxed(b"mdat", b"two"));
        assert_eq!(media, expected);
    }

    #[test]
    fn the_reader_serves_fragments_in_order_and_ends_when_the_fetcher_stops() {
        let (sender, fragments) = bounded(QUEUED_FRAGMENTS);
        sender.send(b"first".to_vec()).unwrap();
        sender.send(b"second".to_vec()).unwrap();
        drop(sender);
        let mut reader = LiveAudioReader {
            fragments,
            current: Vec::new(),
            position: 0,
        };
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"firstsecond");
        assert_eq!(reader.read(&mut [0; 4]).unwrap(), 0);
    }

    #[test]
    fn a_forbidden_live_poll_reports_the_refusal_that_starts_a_new_resolution() {
        assert_eq!(
            refusal_message(reqwest::StatusCode::FORBIDDEN),
            crate::playback::engine::googlevideo_refusal_message()
        );
        assert!(
            refusal_message(reqwest::StatusCode::INTERNAL_SERVER_ERROR)
                .starts_with("live poll refused")
        );
    }

    #[test]
    fn a_live_edge_that_stays_put_for_a_minute_is_stalled() {
        let since = Instant::now();
        assert!(!edge_stalled(since, since + Duration::from_secs(59)));
        assert!(edge_stalled(since, since + Duration::from_secs(60)));
    }

    #[test]
    fn a_box_claiming_more_bytes_than_the_response_holds_is_ignored() {
        // A 64-bit size of u64::MAX: adding it to the offset would overflow.
        let mut data = vec![0, 0, 0, 1];
        data.extend_from_slice(b"moof");
        data.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(top_level_boxes(&data).is_empty());
        // Parsing stops at the broken box, so a valid box after it is not read.
        let mut mixed = data.clone();
        mixed.extend(boxed(b"mdat", b"audio"));
        assert!(top_level_boxes(&mixed).is_empty());
        assert_eq!(top_level_boxes(&boxed(b"mdat", b"audio")).len(), 1);
    }

    #[test]
    fn the_reader_refuses_to_seek() {
        let (_sender, fragments) = bounded::<Vec<u8>>(1);
        let mut reader = LiveAudioReader {
            fragments,
            current: Vec::new(),
            position: 0,
        };
        assert!(reader.seek(SeekFrom::Start(0)).is_err());
    }

    #[test]
    fn playback_asks_the_live_edge_first_and_then_the_next_sequence() {
        assert_eq!(request_for(None, Some(9)), None);
        assert_eq!(request_for(Some(10), Some(10)), Some(10));
        assert_eq!(request_for(Some(10), None), Some(10));
    }

    #[test]
    fn playback_jumps_to_the_head_only_beyond_the_lag_limit() {
        let next = 10;
        assert_eq!(
            request_for(Some(next), Some(next + MAX_LAG_FRAGMENTS)),
            Some(next)
        );
        assert_eq!(
            request_for(Some(next), Some(next + MAX_LAG_FRAGMENTS + 1)),
            Some(next + MAX_LAG_FRAGMENTS + 1)
        );
    }

    #[test]
    fn an_answer_is_accepted_for_the_fragment_asked_for_and_judged_otherwise() {
        assert_eq!(verdict(10, 10), Verdict::Accept);
        assert_eq!(verdict(10, 13), Verdict::Gap);
        assert_eq!(verdict(10, 9), Verdict::Behind);
    }

    #[test]
    fn the_sequence_parameter_replaces_an_earlier_one_and_keeps_the_other_text() {
        let url = reqwest::Url::parse(
            "https://media.example.test/videoplayback?itag=140&sig=a%2Bb&live=1",
        )
        .unwrap();
        assert_eq!(
            sequenced_url(&url, 7).unwrap().as_str(),
            "https://media.example.test/videoplayback?itag=140&sig=a%2Bb&live=1&sq=7"
        );
        let earlier =
            reqwest::Url::parse("https://media.example.test/videoplayback?sq=3&itag=140").unwrap();
        assert_eq!(
            sequenced_url(&earlier, 7).unwrap().as_str(),
            "https://media.example.test/videoplayback?itag=140&sq=7"
        );
        let bare = reqwest::Url::parse("https://media.example.test/videoplayback").unwrap();
        assert_eq!(
            sequenced_url(&bare, 7).unwrap().as_str(),
            "https://media.example.test/videoplayback?sq=7"
        );
    }

    #[test]
    fn the_sequence_headers_are_read_as_numbers() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(SEQUENCE_HEADER, HeaderValue::from_static("171627"));
        headers.insert(
            HEAD_SEQUENCE_HEADER,
            HeaderValue::from_static("not a number"),
        );
        assert_eq!(header_number(&headers, SEQUENCE_HEADER), Some(171_627));
        assert_eq!(header_number(&headers, HEAD_SEQUENCE_HEADER), None);
        assert_eq!(header_number(&headers, "x-missing"), None);
    }
}
