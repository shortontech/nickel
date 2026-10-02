// @jsx h
export function About() {
    const system = nickel.system.get();
    return h(Column, { className: "settings-page" },
        h(Text, { className: "settings-title" }, "About Nickel"),
        h(Text, { wrap: true }, "Nickel desktop shell"),
        h(Text, null, "Version: " + (system.version || "Unavailable")),
        h(Text, null, "Platform: " + (system.available ? system.platform + " · " + system.architecture : "Unavailable")));
}
registerSettingsPage({ id: "about", group: "Shell", label: "About", description: "Nickel version and platform", component: About });
