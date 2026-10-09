//! Explicit opt-in read-only catalog checks. No credentials or raw responses are persisted.
use crabytube::music::{
    BrowsePage, CatalogFilter, HomeItemKind, MusicService, YouTubeMusicService,
};
use iced::widget::image;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires live anonymous YouTube Music; run explicitly"]
async fn several_real_home_collections_have_playable_tracks() {
    let service = YouTubeMusicService::new();
    let shelves = service.home().await.expect("live Home");
    let mut checked = 0;
    for item in shelves.iter().flat_map(|shelf| &shelf.items) {
        if !matches!(item.kind, HomeItemKind::Playlist | HomeItemKind::Mix) {
            continue;
        }
        let Some(target) = item.browse_target.as_ref() else {
            continue;
        };
        if !target.is_public_collection() {
            continue;
        }
        let BrowsePage::Collection(data) = service.browse(target).await.expect("live browse")
        else {
            panic!("public playlist was not parsed as a collection");
        };
        assert!(!data.title.is_empty());
        assert!(!data.tracks.is_empty());
        assert!(
            data.tracks
                .iter()
                .all(|entry| !entry.track.video_id.is_empty() && !entry.track.title.is_empty())
        );
        println!(
            "{} → {}: {} playable tracks; owner={}; artwork={}; continuation={}",
            item.title,
            data.title,
            data.tracks.len(),
            data.owner.is_some(),
            data.thumbnail_url.is_some(),
            data.continuation.is_some()
        );
        checked += 1;
        if checked == 4 {
            break;
        }
    }
    assert_eq!(checked, 4, "need several distinct real Home collections");
}

#[tokio::test]
#[ignore = "requires live anonymous YouTube Music; run explicitly"]
async fn real_search_result_can_seed_radio_tracks() {
    let service = YouTubeMusicService::new();
    let items = service
        .search_catalog("Daft Punk")
        .await
        .expect("live search");
    let seed = items
        .iter()
        .find_map(|item| item.track.as_ref())
        .expect("search returns playable songs");
    let radio = service.radio(&seed.video_id).await.expect("live radio");
    assert!(!radio.is_empty(), "radio should return follow-up tracks");
    println!(
        "live search: {} catalog items; radio: {} follow-up tracks",
        items.len(),
        radio.len()
    );
}

#[tokio::test]
#[ignore = "requires live anonymous YouTube Music and artwork CDN; run explicitly"]
async fn real_home_and_explore_expand_and_artwork_decodes() {
    fn shelf_key(shelf: &crabytube::music::HomeShelf) -> (String, String) {
        let first = shelf.items.first();
        let item_id = first
            .and_then(|item| item.browse_id.clone())
            .or_else(|| {
                first
                    .and_then(|item| item.track.as_ref())
                    .map(|track| track.video_id.clone())
            })
            .or_else(|| first.map(|item| item.title.trim().to_lowercase()))
            .unwrap_or_default();
        (shelf.title.trim().to_lowercase(), item_id)
    }
    let service = YouTubeMusicService::new();
    let home = service.home().await.expect("primary live Home");
    let home_more = service.home_more().await.expect("supplemental live Home");
    let mut all_home = home.clone();
    all_home.extend(home_more.clone());

    let shelf_count = |shelves: &[crabytube::music::HomeShelf]| shelves.len();
    let unique_home_shelves = all_home.iter().map(shelf_key).collect::<HashSet<_>>().len();
    println!(
        "live Home: {} primary + {} supplemental = {} unique shelves",
        shelf_count(&home),
        shelf_count(&home_more),
        unique_home_shelves,
    );
    assert!(!home.is_empty());
    assert!(!home_more.is_empty(), "Home should add live catalog feeds");

    let urls = all_home
        .iter()
        .flat_map(|shelf| &shelf.items)
        .filter_map(|item| item.thumbnail_url.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .take(4)
        .collect::<Vec<_>>();
    assert!(
        urls.len() >= 2,
        "live feed should contain multiple artworks"
    );
    // Each size variant must decode within its own pixel bound.
    let mut thumb_ok = 0;
    let mut large_ok = 0;
    for url in urls {
        if let Some(handle) =
            crabytube::artwork::fetch_sized(url.clone(), CancellationToken::new(), 128).await
        {
            match handle {
                image::Handle::Rgba { width, height, .. } => {
                    assert!(
                        width <= 128 && height <= 128,
                        "thumb variant must stay within 128px, got {width}x{height}"
                    );
                    assert_eq!(
                        crabytube::artwork::handle_bytes(&handle),
                        width as usize * height as usize * 4
                    );
                    thumb_ok += 1;
                }
                _ => panic!("thumb variant must be decoded RGBA"),
            }
        }
        if let Some(handle) = crabytube::artwork::fetch(url, CancellationToken::new()).await {
            match handle {
                image::Handle::Rgba { width, height, .. } => {
                    assert!(
                        width <= 640 && height <= 640,
                        "large variant must stay within 640px, got {width}x{height}"
                    );
                    large_ok += 1;
                }
                _ => panic!("large variant must be decoded RGBA"),
            }
        }
    }
    println!("real artwork thumb decoded: {thumb_ok}; large decoded: {large_ok}");
    assert!(
        thumb_ok >= 2,
        "multiple live thumbs should decode within 128px"
    );
    assert!(
        large_ok >= 2,
        "multiple live large artworks should decode within 640px"
    );
}

#[tokio::test]
#[ignore = "requires live anonymous YouTube Music; run explicitly"]
async fn remote_category_search_returns_matching_catalog_kinds() {
    let service = YouTubeMusicService::new();
    for filter in [
        CatalogFilter::Songs,
        CatalogFilter::Albums,
        CatalogFilter::Artists,
        CatalogFilter::Playlists,
    ] {
        // YouTube Music does not consistently return playlists for an artist
        // query, even when the remote playlist filter is accepted. Use a
        // known public playlist phrase for that result kind.
        let query = if filter == CatalogFilter::Playlists {
            "Pop's Biggest Hits"
        } else {
            "Daft Punk"
        };
        let items = service
            .search_catalog_filtered(query, filter)
            .await
            .expect("live category search");
        assert!(
            !items.is_empty(),
            "category search should return real additional items: {filter:?}"
        );
        assert!(
            items.iter().all(|item| filter.matches(item.kind)),
            "category result contained an unexpected kind: {filter:?}"
        );
        println!(
            "live category {filter:?} for {query:?}: {} matching items",
            items.len()
        );
    }
}
