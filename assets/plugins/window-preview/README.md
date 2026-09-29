# Window preview plugin

`main.jsx` defines the preview and task switcher presentation. Its fixed
`<Window>` fills the host's current size, which varies with the number of
windows. `ui.css` styles the same native controls in both contexts. Nickel
supplies window thumbnails and validates each requested window action.

Regenerate `main.js` with:

```sh
tsc --allowJs --checkJs false --noCheck --jsx react --jsxFactory h \
  --target ES2020 --lib ES2020 --module none \
  --outDir assets/plugins/window-preview assets/plugins/window-preview/main.jsx
```
