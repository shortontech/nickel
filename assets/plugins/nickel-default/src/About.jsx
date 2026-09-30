// @jsx h
export function About() {
    const system = nickel.system.get();
    return <Column className="settings-page">
        <Text className="settings-title">About Nickel</Text>
        <Text wrap={true}>Nickel desktop shell</Text>
        <Text>{"Version: " + (system.version || "Unavailable")}</Text>
        <Text>{"Platform: " + (system.available ? system.platform + " · " + system.architecture : "Unavailable")}</Text>
    </Column>;
}
registerSettingsPage({id:"about",group:"Shell",label:"About",description:"Nickel version and platform",component:About});
