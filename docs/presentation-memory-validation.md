# Presentation memory accounting and validation

The compositor software compatibility path owns one reusable Smithay
`MemoryRenderBuffer` per visible fallback surface. Compatible frames retain the
buffer identity; unchanged frames do not draw into it. Changed pixels are
premultiplied only inside outward-rounded, clipped physical damage. A new buffer,
resize, or scale change forces a complete repaint. GPU recovery and owner suspension
drop the fallback storage.

Smithay at revision `e3d461a` serializes memory access with its render context,
keeps damage/import state per renderer context, and gives imported render elements
an `Arc` snapshot of CPU pixels. Writes use copy-on-write if an element still retains
that snapshot. Nickel does not maintain an additional buffer pool. A retained element
can therefore temporarily retain another full CPU pixel payload; this is not included
in the owning surface's current buffer bytes. Driver textures and retired element
snapshots are not exposed by the Smithay API and must be measured separately. The
snapshot regression imports a frame, retains it across partial updates, and verifies
that its bytes remain unchanged while the resource identity is reused.

`fallback_converted_bytes` counts conversion work. `fallback_upload_damage_bytes`
counts the bounding damage submitted to Smithay, which merges dirty regions for
texture updates. It is not an actual upload-byte measurement: first imports in another
renderer context require a complete upload and multiple unpresented updates may be
combined. Creation/reuse and full/partial repaint counters describe host buffer work.

Image source keys retain content hashes, dimensions, generation, and selected-density
identity, but exclude destination scale. All image buffers use premultiplied Abgr8888;
Smithay separates renderer-context imports for the shared CPU buffer. Text remains
scale-specific because its pixels change with scale.

Private text scratch capacity is reported separately from private derived cache bytes
and shared texture bytes. Suspension releases the scratch allocation to the software
renderer's four-byte sentinel and clears private caches, preserving shared textures
and the process font owner. Scratch exceeding 8 MiB is retired immediately after its
pixels are copied into an independently owned text texture, including while visible.

Automated checks live in `session::internal_ui::tests`: partial alpha/motion updates
at fractional scale against a fresh full raster, imported snapshot ownership, clipped
disjoint damage, incompatible scale/resize and suspend/reshow, destination scale image
reuse, and repeated text-owner suspension with another visible consumer.

Run the reproducible CPU workload with:

```sh
cargo test -p nickel --lib --release measure_fallback_damage_workloads -- --ignored --nocapture
```

This reports 120-frame partial/full workloads at 1080p and 4K, current owned CPU bytes,
conversion bytes, submitted damage, buffer creations, and elapsed time. It does not
create a native graphics context and cannot establish actual VRAM or driver upload
counts. Native/nested Linux mixed-DPI visuals, context replacement, import failures,
and end-to-end first-frame latency remain live acceptance checks.
