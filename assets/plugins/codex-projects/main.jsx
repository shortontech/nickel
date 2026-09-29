// @jsx h
// Project roots stay in the host. This view receives only names and opaque IDs.
function App() {
    const data = nickel.data || {};
    const availableProjects = data.projects || [];
    const [query, setQuery] = useState("");
    const normalized = query.trim().toLowerCase();
    const projects = availableProjects.filter(project =>
        !normalized || project.name.toLowerCase().includes(normalized) || project.id.toLowerCase().includes(normalized));
    const message = data.status === "loading" ? "Loading projects…"
        : data.status === "sign-in-required" ? "Sign in to Codex to load projects"
        : data.status === "unavailable" ? "Codex is unavailable"
        : data.status === "disconnected" ? "Codex is disconnected"
        : data.status === "incompatible" ? "Codex is incompatible"
        : availableProjects.length === 0 ? "No projects available"
        : projects.length === 0 ? "No matching projects" : "Choose a project";
    return <FixedWindow width={520} height={680} className="codex-projects">
        <Column>
            <Row>
                <Text>Codex projects</Text>
                <Button id="codex-project-refresh" onClick={() => nickel.request({type: "codex-project-refresh"})}>Refresh</Button>
                <Button id="codex-project-close" onClick={() => nickel.request({type: "codex-project-close"})}>Close</Button>
            </Row>
            <Text>{message}</Text>
            <TextField id="codex-project-query" value={query} placeholder="Search projects"
                onChange={value => setQuery(value)} />
            <ScrollView id="codex-project-scroll" height={550}>
                <Column>
                    {projects.map(project => <Button key={project.id} id={"codex-project-" + project.id}
                        onClick={() => nickel.request({type: "codex-project-open", id: project.id, revision: data.revision})}>
                        {project.name}
                    </Button>)}
                </Column>
            </ScrollView>
        </Column>
    </FixedWindow>;
}
