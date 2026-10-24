//! Long tracks such as podcast episodes. YouTube sends their audio as an MP4 of fragments of a few
//! seconds each, listed in a segment index (`sidx`) near the start. Told that such a file is
//! seekable, the decoder reads across every fragment before its first sample, which for an episode
//! of hours means downloading most of it. These files are therefore decoded as a stream, from the
//! index: the decoder reads the fragments as they play, and a seek starts a new decoder at the
//! fragment that holds the position.
use std::{
    io::{self, Read, Seek, SeekFrom},
    num::{NonZeroU16, NonZeroU32},
    sync::{Arc, Mutex},
    time::Duration,
};

use rodio::{Decoder, Source, decoder::DecoderError};

/// The boxes before the first fragment are a few kilobytes; a file whose header is larger is
/// decoded as a whole file instead.
const MAX_HEADER_BYTES: u64 = 4 * 1024 * 1024;

/// Where each fragment of a long MP4 starts, from its segment index.
#[derive(Debug)]
pub(super) struct SegmentIndex {
    /// The file before its first fragment: its `ftyp` and `moov` boxes and the index.
    header: Arc<[u8]>,
    /// Each fragment's offset in the file, with the time it starts at.
    fragments: Vec<(u64, Duration)>,
}

impl SegmentIndex {
    /// Reads the boxes before the first fragment and the index among them. `None` when the file has
    /// no index, or its first fragment is not where the index says. The reader is left at its start
    /// either way.
    pub(super) fn read<R: Read + Seek>(reader: &mut R) -> io::Result<Option<Self>> {
        let index = Self::read_header(reader);
        reader.seek(SeekFrom::Start(0))?;
        Ok(index)
    }

    fn read_header<R: Read>(reader: &mut R) -> Option<Self> {
        let mut header = Vec::new();
        let mut index = None;
        loop {
            let mut head = [0_u8; 16];
            reader.read_exact(&mut head[..8]).ok()?;
            let kind = [head[4], head[5], head[6], head[7]];
            // The first fragment starts here.
            if &kind == b"moof" || &kind == b"mdat" {
                break;
            }
            let (size, head_len) = match u32::from_be_bytes([head[0], head[1], head[2], head[3]]) {
                // The 64-bit size follows the type.
                1 => {
                    reader.read_exact(&mut head[8..16]).ok()?;
                    (u64::from_be_bytes(head[8..16].try_into().ok()?), 16)
                }
                // A box that runs to the end of the file before any fragment: none to index.
                0 => return None,
                size => (u64::from(size), 8),
            };
            if size < head_len as u64 || header.len() as u64 + size > MAX_HEADER_BYTES {
                return None;
            }
            let start = header.len();
            header.extend_from_slice(&head[..head_len]);
            header.resize(start + usize::try_from(size).ok()?, 0);
            reader.read_exact(&mut header[start + head_len..]).ok()?;
            if &kind == b"sidx" {
                index = Some(parse_sidx(
                    &header[start + head_len..],
                    header.len() as u64,
                )?);
            }
        }
        let fragments = index?;
        // The index counts from the end of its own box; the first fragment must start there.
        if fragments.first()?.0 != header.len() as u64 {
            return None;
        }
        Some(Self {
            header: header.into(),
            fragments,
        })
    }

    /// The fragment that holds `position`: its offset in the file and the time it starts at.
    fn fragment_at(&self, position: Duration) -> (u64, Duration) {
        let after = self
            .fragments
            .partition_point(|(_, start)| *start <= position);
        self.fragments[after.saturating_sub(1)]
    }
}

/// The fragments a segment index lists: each one's offset in the file and start time. `end` is
/// where the index's box ends in the file. Only an index of fragments is read; one that points to
/// further indexes is not.
fn parse_sidx(body: &[u8], end: u64) -> Option<Vec<(u64, Duration)>> {
    let mut at = 0_usize;
    let mut take = |len: usize| {
        let bytes = body.get(at..at + len)?;
        at += len;
        Some(bytes)
    };
    let number = |bytes: &[u8]| {
        bytes
            .iter()
            .fold(0_u64, |value, byte| value << 8 | u64::from(*byte))
    };
    let version = take(4)?[0];
    let _reference_id = take(4)?;
    let timescale = number(take(4)?);
    let wide = if version == 0 { 4 } else { 8 };
    let _earliest = take(wide)?;
    let first_offset = number(take(wide)?);
    let _reserved = take(2)?;
    let count = number(take(2)?);
    if timescale == 0 || count == 0 {
        return None;
    }
    let mut offset = end.checked_add(first_offset)?;
    let mut ticks = 0_u64;
    let mut fragments = Vec::with_capacity(usize::try_from(count).ok()?);
    for _ in 0..count {
        let reference = number(take(4)?);
        let duration = number(take(4)?);
        let _sap = take(4)?;
        // The top bit marks a reference to another index.
        if reference & 0x8000_0000 != 0 {
            return None;
        }
        let seconds = ticks / timescale;
        let nanos = (ticks % timescale) * 1_000_000_000 / timescale;
        fragments.push((offset, Duration::new(seconds, u32::try_from(nanos).ok()?)));
        offset = offset.checked_add(reference & 0x7fff_ffff)?;
        ticks = ticks.checked_add(duration)?;
    }
    Some(fragments)
}

/// A fragmented file as one decoder reads it: the file's header, then the file from one of its
/// fragments to the end. The decoders of a track read the one download, which keeps its position.
pub(super) struct FragmentReader<R> {
    download: Arc<Mutex<(R, u64)>>,
    header: Arc<[u8]>,
    /// Where in the file the fragments this reader shows start.
    from: u64,
    /// The position in what this reader shows.
    position: u64,
}

impl<R: Read + Seek> Read for FragmentReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let header_len = self.header.len() as u64;
        if self.position < header_len {
            let start = usize::try_from(self.position).map_err(io::Error::other)?;
            let count = buf.len().min(self.header.len() - start);
            buf[..count].copy_from_slice(&self.header[start..start + count]);
            self.position += count as u64;
            return Ok(count);
        }
        let at = self.from + (self.position - header_len);
        let mut download = self
            .download
            .lock()
            .map_err(|_| io::Error::other("the download was left half-read"))?;
        let (reader, reader_at) = &mut *download;
        if *reader_at != at {
            reader.seek(SeekFrom::Start(at))?;
            *reader_at = at;
        }
        let count = reader.read(buf)?;
        *reader_at += count as u64;
        self.position += count as u64;
        Ok(count)
    }
}

impl<R> Seek for FragmentReader<R> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let position = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            // The decoder reads a fragmented file as a stream, without its length.
            SeekFrom::End(_) => None,
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))?;
        self.position = position;
        Ok(position)
    }
}

/// The decoder of a track: a whole file, or a long fragmented one read through its segment index.
pub(super) enum TrackDecoder<R: Read + Seek + Send + Sync + 'static> {
    Whole(Decoder<R>),
    Fragmented {
        decoder: Decoder<FragmentReader<R>>,
        download: Arc<Mutex<(R, u64)>>,
        index: SegmentIndex,
    },
}

impl<R: Read + Seek + Send + Sync + 'static> TrackDecoder<R> {
    /// The decoder of a download. One longer than `whole_up_to` bytes is read through its segment
    /// index when it has one.
    pub(super) fn open(
        mut reader: R,
        byte_len: Option<u64>,
        whole_up_to: u64,
    ) -> Result<Self, DecoderError> {
        if byte_len.is_some_and(|length| length > whole_up_to) {
            let index = SegmentIndex::read(&mut reader)
                .map_err(|error| DecoderError::IoError(error.to_string()))?;
            if let Some(index) = index {
                let download = Arc::new(Mutex::new((reader, 0)));
                let decoder = fragment_decoder(&download, &index, index.fragments[0].0)?;
                return Ok(Self::Fragmented {
                    decoder,
                    download,
                    index,
                });
            }
        }
        let builder = Decoder::builder()
            .with_data(reader)
            .with_mime_type("audio/mp4")
            .with_seekable(byte_len.is_some());
        let builder = match byte_len {
            Some(length) => builder.with_byte_len(length),
            None => builder,
        };
        builder.build().map(Self::Whole)
    }

    pub(super) fn sample_rate(&self) -> NonZeroU32 {
        match self {
            Self::Whole(decoder) => decoder.sample_rate(),
            Self::Fragmented { decoder, .. } => decoder.sample_rate(),
        }
    }

    pub(super) fn channels(&self) -> NonZeroU16 {
        match self {
            Self::Whole(decoder) => decoder.channels(),
            Self::Fragmented { decoder, .. } => decoder.channels(),
        }
    }

    pub(super) fn next(&mut self) -> Option<f32> {
        match self {
            Self::Whole(decoder) => decoder.next(),
            Self::Fragmented { decoder, .. } => decoder.next(),
        }
    }

    /// Moves to `target`. A fragmented file gets a new decoder from the fragment that holds the
    /// target, which decodes and drops what comes before the target in that fragment.
    pub(super) fn try_seek(&mut self, target: Duration) -> Result<(), String> {
        match self {
            Self::Whole(decoder) => decoder.try_seek(target).map_err(|error| error.to_string()),
            Self::Fragmented {
                decoder,
                download,
                index,
            } => {
                let (from, start) = index.fragment_at(target);
                let mut fresh =
                    fragment_decoder(download, index, from).map_err(|error| error.to_string())?;
                let per_second =
                    f64::from(fresh.sample_rate().get()) * f64::from(fresh.channels().get());
                let skip = ((target - start).as_secs_f64() * per_second) as u64;
                // Whole frames, so the channels stay in place.
                let skip = skip - skip % u64::from(fresh.channels().get());
                for _ in 0..skip {
                    if fresh.next().is_none() {
                        break;
                    }
                }
                *decoder = fresh;
                Ok(())
            }
        }
    }
}

/// A decoder of the fragments from the one at `from` in the file.
fn fragment_decoder<R: Read + Seek + Send + Sync + 'static>(
    download: &Arc<Mutex<(R, u64)>>,
    index: &SegmentIndex,
    from: u64,
) -> Result<Decoder<FragmentReader<R>>, DecoderError> {
    let reader = FragmentReader {
        download: Arc::clone(download),
        header: Arc::clone(&index.header),
        from,
        position: 0,
    };
    Decoder::builder()
        .with_data(reader)
        .with_mime_type("audio/mp4")
        .with_seekable(false)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A segment index of fragments of these sizes and durations, at a timescale of 1000.
    fn sidx(fragments: &[(u32, u32)]) -> Vec<u8> {
        let mut body = vec![0, 0, 0, 0];
        body.extend_from_slice(&1_u32.to_be_bytes());
        body.extend_from_slice(&1000_u32.to_be_bytes());
        body.extend_from_slice(&0_u32.to_be_bytes());
        body.extend_from_slice(&0_u32.to_be_bytes());
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&(fragments.len() as u16).to_be_bytes());
        for (size, duration) in fragments {
            body.extend_from_slice(&size.to_be_bytes());
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&0x9000_0000_u32.to_be_bytes());
        }
        boxed(b"sidx", &body)
    }

    fn file(fragments: &[(u32, u32)]) -> Vec<u8> {
        let mut file = boxed(b"ftyp", b"dash");
        file.extend(boxed(b"moov", &[7; 20]));
        file.extend(sidx(fragments));
        for (index, (size, _)) in fragments.iter().enumerate() {
            let body = vec![index as u8; *size as usize - 8];
            file.extend(boxed(b"moof", &body));
        }
        file
    }

    #[test]
    fn the_index_gives_each_fragment_its_offset_and_start() {
        let data = file(&[(40, 9000), (50, 9000), (30, 4500)]);
        let mut reader = io::Cursor::new(data.clone());
        let index = SegmentIndex::read(&mut reader).unwrap().unwrap();
        assert_eq!(reader.position(), 0);
        let first = index.header.len() as u64;
        assert_eq!(&data[..index.header.len()], &index.header[..]);
        assert_eq!(
            index.fragments,
            vec![
                (first, Duration::ZERO),
                (first + 40, Duration::from_secs(9)),
                (first + 90, Duration::from_secs(18)),
            ]
        );
        assert_eq!(index.fragment_at(Duration::from_secs(0)).0, first);
        assert_eq!(index.fragment_at(Duration::from_millis(8_999)).0, first);
        assert_eq!(index.fragment_at(Duration::from_secs(9)).0, first + 40);
        assert_eq!(index.fragment_at(Duration::from_secs(600)).0, first + 90);
    }

    #[test]
    fn a_file_without_an_index_is_read_whole() {
        let mut data = boxed(b"ftyp", b"dash");
        data.extend(boxed(b"moov", &[7; 20]));
        data.extend(boxed(b"mdat", &[1; 64]));
        let mut reader = io::Cursor::new(data);
        assert!(SegmentIndex::read(&mut reader).unwrap().is_none());
        assert_eq!(reader.position(), 0);
    }

    #[test]
    fn an_index_that_misplaces_the_first_fragment_is_not_used() {
        let mut data = boxed(b"ftyp", b"dash");
        data.extend(sidx(&[(40, 9000)]));
        data.extend(boxed(b"free", &[0; 8]));
        data.extend(boxed(b"moof", &[0; 32]));
        assert!(
            SegmentIndex::read(&mut io::Cursor::new(data))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_reader_shows_the_header_then_the_file_from_its_fragment() {
        let data = file(&[(40, 9000), (50, 9000)]);
        let index = SegmentIndex::read(&mut io::Cursor::new(data.clone()))
            .unwrap()
            .unwrap();
        let download = Arc::new(Mutex::new((io::Cursor::new(data.clone()), 0)));
        let second = index.fragments[1].0;
        let mut reader = FragmentReader {
            download: Arc::clone(&download),
            header: Arc::clone(&index.header),
            from: second,
            position: 0,
        };
        let mut shown = Vec::new();
        reader.read_to_end(&mut shown).unwrap();
        let mut expected = index.header.to_vec();
        expected.extend_from_slice(&data[second as usize..]);
        assert_eq!(shown, expected);
        // A second reader of the same download starts where it should, whatever the first read.
        let mut first = FragmentReader {
            download,
            header: Arc::clone(&index.header),
            from: index.fragments[0].0,
            position: index.header.len() as u64,
        };
        let mut head = [0_u8; 8];
        first.read_exact(&mut head).unwrap();
        assert_eq!(&head[4..], b"moof");
        assert_eq!(&head[..4], &40_u32.to_be_bytes());
    }
}
