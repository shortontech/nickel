// Host-owned Settings/provider client for admitted component linking.
function __nickelCompositionSettingsPages() {
    const result = readPluginSettingsPages();
    for (const entry of result.pages) {
        if (typeof entry.component !== 'function') entry.component = __nickelComponentProxy({settingPage:{provider:entry.providerPackage,id:entry.id}});
    }
    return result;
}
const __nickelCompositionClient = Object.freeze({...nickel, get data() { return nickel.data; },
    component(contract) {
        if (typeof contract !== 'string') throw TypeError('invalid component contract');
        return __nickelComponentProxy({contract});
    },
    contributions(collection) {
        if (typeof collection !== 'string') throw TypeError('invalid contribution collection');
        return Object.freeze((__nickelContributionCatalog[collection] || []).map(entry => Object.freeze({
            ...entry, component: __nickelComponentProxy({contribution:entry.key})
        })));
    }
});
