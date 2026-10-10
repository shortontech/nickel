// Host-owned Settings/provider client for admitted component linking.
function __nickelCompositionSettingsPages() {
    const result = readPluginSettingsPages();
    for (const entry of result.pages) {
        if (typeof entry.component !== 'function') entry.component = __twinkleComponentProxy({settingPage:{provider:entry.providerPackage,id:entry.id}});
    }
    return result;
}
const __nickelCompositionClient = Object.freeze({...nickel, get data() { return nickel.data; },
    component(contract) { return __twinkleCompositionClient.component(contract); },
    contributions(collection) { return __twinkleCompositionClient.contributions(collection); }
});
