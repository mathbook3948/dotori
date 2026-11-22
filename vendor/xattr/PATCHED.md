# xattr 0.2.3, patched

Copied from crates.io `xattr` 0.2.3 (MIT OR Apache-2.0, by Steven Allen). gpui 0.2.2 depends on it
through `zed-async-tar`, and it no longer builds against libc 0.2.191, which dropped `ENOATTR` on
Linux. The only change is in `src/sys/mod.rs`: Linux and Android use `ENODATA`, the same value.
Remove this copy once gpui depends on a fixed release.
