# Runtime cache limits

Nickel treats rendered assets and live window previews as disposable consequences of authoritative
application and compositor state. The caches have hard entry limits so changing search results,
window titles, icons, scales, and preview targets cannot grow a desktop session without bound.

| Cache | Limit | Eviction and diagnostics |
| --- | ---: | --- |
| Launcher icons, including negative lookups | 512 | Oldest inserted entry; `LauncherIconCache::diagnostics` |
| Shared CPU text rasters | 512 | Oldest inserted entry; `TextAssetCache::diagnostics` |
| Shared decoded images | 256 | Oldest inserted image and all of its scaled variants; `ImageAssetCache::diagnostics` |
| Shared scaled image variants | 512 | Oldest inserted variant; `ImageAssetCache::diagnostics` |
| SDL text layouts and rasters | 2,048 each | Whole-cache reset at the limit |
| SDL uploaded image textures | 512 | Whole-cache reset with explicit texture destruction |
| Compositor window previews | 1,024 | Removed when no longer requested or when the window closes |
| Notification history | 100 entries; 256-character app, 512-character summary, 4,096-character body, 3 actions | Oldest notification is closed with the expired reason |
| Server-decoration title rasters | 128 entries and 16 MiB | Whole-cache reset at either limit; live/peak bytes and activity counters from `titlebar_cache_diagnostics` |

Window shadows are three cheap solid-color descriptors reconstructed from authoritative frame
geometry. They are intentionally not cached or retained between frames.

`nickel-test-input caches` reports the compositor preview entry count, hard limit, and current RGBA
byte total for an explicitly test-controlled nested session. The shared asset and launcher cache APIs
expose entry, capacity, and cumulative eviction counters to application diagnostics without exposing
native handles or retaining duplicate semantic state.

Visible grouped-window previews refresh at most twice per second. Preview frames are fetched by
correlated request ID over the session socket; stale timed-out replies are discarded instead of
being mistaken for the next query. The compositor does not broadcast RGBA frames to subscribers,
so an open preview cannot fill unrelated control or input queues.
