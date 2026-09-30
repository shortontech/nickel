// Ordinary component executed in the contributor's own capability context.
export function ControlSection() {
    return h(Column, {id:"find-apps"},
        h(Text, {}, "Applications"),
        h(Text, {}, "Search Nickel's catalog"),
        h(Button, {onClick:()=>nickel.surfaces.show("launcher")}, "Open launcher"));
}
export default ControlSection;
