# Notification plugin comparison path

`main.jsx` is Nickel's bundled notification view. Generate `main.js` with:

```sh
tsc --allowJs --checkJs false --jsx react --jsxFactory h --target ES2020 \
  --outDir assets/plugins/notification assets/plugins/notification/main.jsx
```

Use `NICKEL_DEV_PLUGIN_NOTIFICATION=1` in nested Nickel to compare it with the
Rust notification view. The host supplies bounded notification data and
revalidates action IDs and keys against the live feed before dispatch.
