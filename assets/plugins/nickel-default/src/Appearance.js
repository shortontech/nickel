// @jsx h
import { fromHsv, toHexColor } from "./colors.js";
import "./styles/appearance.css";
const defaults = { theme: "system", accent_hue: null, accent_intensity: null, reduce_transparency: false, animations: "normal" };
const wallpaperKey = image => image.id;
// Estimates only: native layout corrects these for the current font, width, and scale.
const wallpaperHeight = image => image.previewAsset ? 126 : 36;
export function Appearance() {
    const appearance = nickel.appearance.get();
    const wallpaper = nickel.wallpaper.get();
    const [customOpen, setCustomOpen] = useState(false);
    const [customHue, setCustomHue] = useState("");
    if (!appearance.available)
        return h(Text, { wrap: true }, appearance.reason || "Appearance is unavailable.");
    const configured = appearance.configured;
    const hue = configured.accent_hue ?? appearance.resolved.hue;
    const intensity = configured.accent_intensity ?? appearance.resolved.intensity;
    const set = patch => nickel.appearance.set({ ...configured, ...patch });
    const modes = [{ id: "light", label: "Light" }, { id: "dark", label: "Dark" }, { id: "system", label: "Automatic" }];
    const hueNumber = Number(customHue);
    const validHue = customHue.trim() !== "" && Number.isInteger(hueNumber) && hueNumber >= 0 && hueNumber <= 359;
    return h(Column, { className: "appearance-page" },
        !appearance.writable ? h(Text, null, "Appearance is read only.") : null,
        h(Column, { className: "appearance-card" },
            h(Text, { className: "appearance-heading" }, "Theme"),
            h(Row, { className: "appearance-choices" }, modes.map(mode => h(Button, { key: mode.id, id: "appearance-mode-" + mode.id, className: configured.theme === mode.id ? "appearance-choice selected" : "appearance-choice", state: configured.theme === mode.id ? "selected" : "unselected", disabled: !appearance.writable, onClick: () => set({ theme: mode.id }) }, mode.label)))),
        h(Column, { className: "appearance-card" },
            h(Text, { className: "appearance-heading" }, "Accent color"),
            h(Row, { className: "appearance-swatches" },
                [0, 30, 60, 120, 180, 210, 270, 330].map(value => h(ColorSwatch, { key: value, id: "appearance-accent-" + value, color: toHexColor(fromHsv([value, 0.65, 0.85, 1])), selected: hue === value, accessibilityLabel: "Accent hue " + value + " degrees", onClick: () => { if (appearance.writable)
                        set({ accent_hue: value }); } })),
                h(Button, { id: "appearance-accent-custom", disabled: !appearance.writable, onClick: () => { setCustomHue(String(hue)); setCustomOpen(true); } }, "Custom hue")),
            h(Text, null, "Hue: " + hue + "°"),
            appearance.writable ? h(Slider, { id: "appearance-hue", accessibilityLabel: "Interface hue", min: 0, max: 359, step: 1, value: hue, onChange: value => set({ accent_hue: value }) }) : null,
            h(Text, null, "Color intensity: " + intensity + "%"),
            appearance.writable ? h(Slider, { id: "appearance-intensity", accessibilityLabel: "Color intensity", min: 0, max: 100, step: 1, value: intensity, onChange: value => set({ accent_intensity: value }) }) : null,
            h(Button, { id: "appearance-inherit-accent", disabled: !appearance.writable, onClick: () => set({ accent_hue: null, accent_intensity: null }) }, "Use system accent")),
        h(Column, { className: "appearance-card" },
            h(Text, { className: "appearance-heading" }, "Interface"),
            h(Row, null,
                h(Text, null, "Reduce transparency"),
                h(Switch, { id: "appearance-transparency", accessibilityLabel: "Reduce transparency", state: appearance.writable ? configured.reduce_transparency ? "on" : "off" : configured.reduce_transparency ? "disabled-on" : "disabled-off", onClick: appearance.writable ? () => set({ reduce_transparency: !configured.reduce_transparency }) : undefined })),
            h(Text, null, "Animations"),
            h(Row, { className: "appearance-choices" }, ["off", "reduced", "normal"].map(value => h(Button, { key: value, id: "appearance-animation-" + value, state: configured.animations === value ? "selected" : "unselected", className: configured.animations === value ? "appearance-choice selected" : "appearance-choice", disabled: !appearance.writable, onClick: () => set({ animations: value }) }, value)))),
        h(Column, { className: "appearance-card" },
            h(Text, { className: "appearance-heading" }, "Wallpaper"),
            !wallpaper.available ? h(Text, { wrap: true }, wallpaper.reason || "Wallpaper is unavailable.") : h(Column, null,
                h(Text, null, wallpaper.configured.custom_image_configured ? "Custom image" : "Default wallpaper"),
                h(Row, { className: "appearance-choices" }, ["center", "tile", "stretch", "fit", "span", "fill"].map(position => h(Button, { key: position, id: "appearance-wallpaper-position-" + position, disabled: !wallpaper.writable, state: wallpaper.configured.position === position ? "selected" : "unselected", onClick: () => nickel.wallpaper.setPosition(position) }, position))),
                h(Button, { id: "appearance-wallpaper-choose", disabled: !wallpaper.writable || !wallpaper.chooser?.available || wallpaper.chooser.pending, onClick: () => nickel.wallpaper.chooseImage() }, wallpaper.chooser?.pending ? "Choosing image…" : "Choose image…"),
                wallpaper.chooser?.result ? h(Text, { wrap: true }, wallpaper.chooser.result.reason || ({ applied: "Wallpaper applied", cancelled: "Image choice cancelled" })[wallpaper.chooser.result.status] || "") : null,
                h(VirtualColumn, { id: "appearance-wallpapers", items: wallpaper.images, itemKey: wallpaperKey, itemHeight: wallpaperHeight, overscan: 96, renderItem: (image, index) => h(Column, { key: image.id },
                        image.previewAsset ? h(Image, { asset: image.previewAsset, width: 160, height: 90, fit: "cover" }) : null,
                        h(Button, { key: image.id, id: "appearance-wallpaper-" + image.id, disabled: !wallpaper.writable, onClick: () => nickel.wallpaper.selectImage(image.id) }, (image.label || "Wallpaper " + (index + 1)) + (image.configured ? " · Current" : ""))) }),
                h(Button, { id: "appearance-wallpaper-remove", disabled: !wallpaper.writable, onClick: () => nickel.wallpaper.resetCustomImage() }, "Use default wallpaper"))),
        h(Button, { id: "appearance-reset", disabled: !appearance.writable, onClick: () => nickel.appearance.set(defaults) }, "Reset appearance"),
        h(Dialog, { id: "appearance-custom-hue-dialog", anchor: "appearance-accent-custom", open: customOpen, width: 360, height: 216, onClose: () => setCustomOpen(false) },
            h(Column, { className: "appearance-card" },
                h(Text, { className: "appearance-heading" }, "Custom hue"),
                h(Text, null, "Choose a hue from 0 to 359 degrees."),
                h(TextField, { id: "appearance-custom-hue-input", accessibilityLabel: "Custom hue", value: customHue, onChange: setCustomHue }),
                h(Row, { className: "appearance-choices" },
                    h(Button, { id: "appearance-custom-hue-apply", disabled: !validHue || !appearance.writable, onClick: () => { set({ accent_hue: hueNumber }); setCustomOpen(false); } }, "Apply"),
                    h(Button, { id: "appearance-custom-hue-cancel", onClick: () => setCustomOpen(false) }, "Cancel")))));
}
registerSettingsPage({ id: "appearance", group: "Personalization", label: "Appearance", description: "Theme, accent, interface, and wallpaper", component: Appearance });
