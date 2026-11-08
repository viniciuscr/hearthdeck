# gpui-storage-progress

A throwaway spike: the library sidebar's storage footer, rebuilt with
[gpui](https://gpui.rs) + [`gpui-kit`](https://gpui-kit.com/component/progress)'s
`ProgressCircle` instead of the libcosmic/iced label that ships today.

```sh
cd spikes/gpui-storage-progress
cargo run          # needs a Wayland or X11 session
```

`preview.png` is a capture of the running window, if you would rather look than
build.

The shipped footer (`services/hearthdeck-frontend/src/screens/library/sidebar.rs`)
is a flat `drive-harddisk-solidstate-symbolic` icon beside a caption and a
free-space string. Here the icon becomes a ring filled to the *used*
percentage, the percentage sits inside it, and the caption and free-space
string are unchanged. The figures come from the same `statvfs` call, formatted
by a copy of the same `human_size`, so the two are directly comparable.

This directory is **not** a member of `services/`. It has its own workspace and
lockfile: nothing here reaches the frontend's dependency tree, `just check`, or
the Arch package. Delete the directory and the repository is untouched.

Read `docs/spikes/gpui-progress.md` for what the spike concluded.
