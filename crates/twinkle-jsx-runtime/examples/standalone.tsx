/// <reference path="../types/twinkle.d.ts" />
import './standalone.css';

const initialRows = [{id: 'one', name: 'First'}, {id: 'two', name: 'Second'}];
const selectReducedMotion = (theme: Readonly<TwinkleThemeSnapshot>) => theme.reducedMotion;

function CounterRow({row}: {row: {id: string; name: string}}) {
    const [count,setCount] = useState(0);
    return <Row className="actions"><Text>{row.name}</Text><Button className="action" id={'count-'+row.id} onClick={() => setCount(previous => previous+1)}>{row.name + ': ' + count}</Button></Row>;
}

export default function App() {
    const [query, setQuery] = useState('');
    const [rows, setRows] = useState(initialRows);
    const reducedMotion = useTheme(selectReducedMotion);
    const locale = useSyncExternalStore(TwinkleStores.locale.subscribe, TwinkleStores.locale.getSnapshot);
    const visible = useMemo(() => rows.filter(row => row.name.toLowerCase().includes(query.toLowerCase())), [rows, query]);
    return <Window id="main" width={720} height={480} title="Twinkle TSX" background={0xff20252c}>
        <Column className="app">
            <Text>Twinkle native TSX</Text>
            <TextField className="entry" placeholder="Type to filter" id="query" value={query} onChange={setQuery} accessibilityLabel="Filter rows" />
            <Row className="actions">
                <Button className="action" id="reverse" onClick={() => setRows(previous => [...previous].reverse())}>Reverse rows</Button>
                <Button className="action" id="confirm" onClick={() => twinkle.request({type: 'confirm', query})}>Confirm</Button>
            </Row>
            {visible.map(row => <CounterRow key={row.id} row={row} />)}
            <Text>{locale.tag + ' / reduced motion: ' + String(reducedMotion)}</Text>
        </Column>
    </Window>;
}
