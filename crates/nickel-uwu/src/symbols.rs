//! Exact-module Microsoft symbol resolution for private Windows entry points.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use object::Object;
use pdb::{FallibleIterator, SymbolData};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};

struct ModuleSymbols {
    base: usize,
    manifest: PathBuf,
    pdb: PathBuf,
    resolved: Mutex<HashMap<String, usize>>,
}

static MODULES: OnceLock<Mutex<HashMap<String, Arc<ModuleSymbols>>>> = OnceLock::new();

pub(crate) fn address(module_name: &str, symbol: &str) -> Result<usize, String> {
    let module = module(module_name)?;
    if let Some(rva) = module.resolved.lock().unwrap().get(symbol).copied() {
        return module
            .base
            .checked_add(rva)
            .ok_or("symbol address overflow".into());
    }

    let rva = resolve_pdb_symbol(&module.pdb, symbol)?;
    {
        let mut resolved = module.resolved.lock().unwrap();
        resolved.insert(symbol.to_owned(), rva);
        write_manifest(&module.manifest, &resolved)?;
    }
    module
        .base
        .checked_add(rva)
        .ok_or("symbol address overflow".into())
}

pub(crate) fn rva(module_name: &str, symbol: &str) -> Result<usize, String> {
    let module = module(module_name)?;
    address(module_name, symbol)?
        .checked_sub(module.base)
        .ok_or("symbol precedes its module base".into())
}

fn module(module_name: &str) -> Result<Arc<ModuleSymbols>, String> {
    let modules = MODULES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(module) = modules.lock().unwrap().get(module_name).cloned() {
        return Ok(module);
    }

    let wide: Vec<u16> = module_name.encode_utf16().chain([0]).collect();
    let handle = unsafe { GetModuleHandleW(windows::core::PCWSTR(wide.as_ptr())) }
        .map_err(|error| format!("get loaded module {module_name}: {error}"))?;
    let mut path = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(Some(handle), &mut path) } as usize;
    if length == 0 || length == path.len() {
        return Err(format!("get path for loaded module {module_name}"));
    }
    let image_path = PathBuf::from(String::from_utf16_lossy(&path[..length]));
    let image =
        fs::read(&image_path).map_err(|error| format!("read {}: {error}", image_path.display()))?;
    let pe = object::File::parse(image.as_slice())
        .map_err(|error| format!("parse {}: {error}", image_path.display()))?;
    let codeview = pe
        .pdb_info()
        .map_err(|error| format!("read CodeView record: {error}"))?
        .ok_or_else(|| format!("{} has no CodeView record", image_path.display()))?;
    let pdb_name =
        Path::new(std::str::from_utf8(codeview.path()).map_err(|_| "PDB path is not UTF-8")?)
            .file_name()
            .ok_or("CodeView record has no PDB filename")?
            .to_string_lossy()
            .into_owned();
    let key = symbol_key(codeview.guid(), codeview.age());
    let cache = symbol_cache()?.join(&pdb_name).join(&key);
    let pdb_path = cache.join(&pdb_name);
    let manifest = cache.join("nickel-uwu-symbols.txt");
    if !pdb_path.exists() {
        download_pdb(&pdb_name, &key, &pdb_path)?;
    }
    validate_pdb(&pdb_path, &key, codeview.age())?;
    let resolved = read_manifest(&manifest)?;
    let module = Arc::new(ModuleSymbols {
        base: handle.0 as usize,
        manifest,
        pdb: pdb_path,
        resolved: Mutex::new(resolved),
    });
    modules
        .lock()
        .unwrap()
        .insert(module_name.to_owned(), module.clone());
    Ok(module)
}

fn symbol_cache() -> Result<PathBuf, String> {
    let root = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
    Ok(PathBuf::from(root).join("Nickel").join("symbols"))
}

fn symbol_key(guid: [u8; 16], age: u32) -> String {
    format!(
        "{:08X}{:04X}{:04X}{}{:X}",
        u32::from_le_bytes(guid[0..4].try_into().unwrap()),
        u16::from_le_bytes(guid[4..6].try_into().unwrap()),
        u16::from_le_bytes(guid[6..8].try_into().unwrap()),
        guid[8..]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<String>(),
        age,
    )
}

fn download_pdb(name: &str, key: &str, destination: &Path) -> Result<(), String> {
    let parent = destination.parent().ok_or("PDB cache path has no parent")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("create symbol cache {}: {error}", parent.display()))?;
    let temporary = parent.join(format!("{name}.download"));
    let url = format!("https://msdl.microsoft.com/download/symbols/{name}/{key}/{name}");
    let mut response = ureq::get(&url)
        .call()
        .map_err(|error| format!("download {url}: {error}"))?;
    let mut output = fs::File::create(&temporary)
        .map_err(|error| format!("create {}: {error}", temporary.display()))?;
    io::copy(&mut response.body_mut().as_reader(), &mut output)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    fs::rename(&temporary, destination)
        .map_err(|error| format!("install downloaded PDB {}: {error}", destination.display()))
}

fn validate_pdb(path: &Path, key: &str, image_age: u32) -> Result<(), String> {
    let file = fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut pdb =
        pdb::PDB::open(file).map_err(|error| format!("parse {}: {error}", path.display()))?;
    let info = pdb
        .pdb_information()
        .map_err(|error| format!("read PDB identity: {error}"))?;
    let guid = info.guid.to_string().replace('-', "").to_uppercase();
    let expected_guid = &key[..32];
    if guid != expected_guid || info.age < image_age {
        return Err(format!("cached PDB identity does not match {key}"));
    }
    Ok(())
}

fn resolve_pdb_symbol(path: &Path, wanted: &str) -> Result<usize, String> {
    let file = fs::File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut pdb =
        pdb::PDB::open(file).map_err(|error| format!("parse {}: {error}", path.display()))?;
    let address_map = pdb.address_map().map_err(|error| error.to_string())?;
    let table = pdb.global_symbols().map_err(|error| error.to_string())?;
    let mut symbols = table.iter();
    while let Some(symbol) = symbols.next().map_err(|error| error.to_string())? {
        let (offset, name) = match symbol.parse().map_err(|error| error.to_string())? {
            SymbolData::Public(data) => (data.offset, data.name),
            SymbolData::Procedure(data) => (data.offset, data.name),
            SymbolData::Data(data) => (data.offset, data.name),
            _ => continue,
        };
        if name.as_bytes() == wanted.as_bytes() {
            let rva = offset
                .to_rva(&address_map)
                .ok_or_else(|| format!("symbol {wanted} has no image RVA"))?;
            return Ok(rva.0 as usize);
        }
    }
    Err(format!("symbol {wanted} is absent from {}", path.display()))
}

fn read_manifest(path: &Path) -> Result<HashMap<String, usize>, String> {
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(HashMap::new());
    };
    contents
        .lines()
        .map(|line| {
            let (name, value) = line
                .split_once('\t')
                .ok_or("invalid symbol manifest line")?;
            let rva = usize::from_str_radix(value, 16).map_err(|_| "invalid manifest RVA")?;
            Ok((name.to_owned(), rva))
        })
        .collect()
}

fn write_manifest(path: &Path, symbols: &HashMap<String, usize>) -> Result<(), String> {
    let mut rows: Vec<_> = symbols.iter().collect();
    rows.sort_by_key(|(name, _)| *name);
    let contents = rows
        .into_iter()
        .map(|(name, rva)| format!("{name}\t{rva:x}\n"))
        .collect::<String>();
    fs::write(path, contents)
        .map_err(|error| format!("write symbol manifest {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{address, symbol_key};

    #[test]
    fn formats_microsoft_symbol_store_key() {
        let guid = [
            0xfc, 0x65, 0xa4, 0x09, 0x66, 0xb1, 0xb7, 0x8c, 0x66, 0xf6, 0x03, 0x18, 0x77, 0xbf,
            0xd6, 0x83,
        ];
        assert_eq!(symbol_key(guid, 1), "09A465FCB1668CB766F6031877BFD6831");
    }

    #[test]
    fn resolves_symbols_for_the_installed_twinui() {
        let system_root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let system32 = PathBuf::from(system_root).join("System32");
        let _twinui =
            unsafe { libloading::Library::new(system32.join("twinui.pcshell.dll")) }.unwrap();
        let twinui_symbols = [
            "?CreateImmersiveShellController@CImmersiveShellBuilder@@UEAAJPEAPEAUIImmersiveShellController@@@Z",
            "?SetShellScenario@CImmersiveShellBuilder@@UEAAJW4ImmersiveShellScenario@@@Z",
            "?ShouldCreateComponent@CImmersiveShellCreationBehavior@@UEAAJIPEAHPEAU_GUID@@@Z",
            "?WindowDiscoveredFromShellHook@UwpWindowWrapperBase@@UEAAXPEAUHWND__@@@Z",
            "?VisibilityChanged@UwpWindowWrapperBase@@UEAAXW4EventPhase@@W4Visibility@@@Z",
            "?HandlePresentationReadinessChange@UwpWindowWrapperBase@@UEAAX_K@Z",
            "?SetShellCloak@UwpWindowWrapperBase@@UEAAJW4__MIDL___MIDL_itf_privilegedoperations_0000_0002_0001@@@Z",
            "?SetForegroundWindow@UwpWindowWrapperBase@@UEAAJXZ",
            "?SetFrameWindow@UwpWindowWrapperBase@@UEAAJPEAUHWND__@@W4__MIDL___MIDL_itf_ntuserviewmanagerinterop_0000_0002_0001@@@Z",
            "?SetSize@UwpWindowWrapperBase@@UEAAJUSize@Foundation@Windows@@@Z",
            "?SetPosition@CApplicationFrameWrapper@@UEAAJPEAUIApplicationViewPosition@@@Z",
            "??$MakeAndInitialize@VCCommonApplicationViewPosition@@UIApplicationViewPosition@@AEAUtagRECT@@@Details@WRL@Microsoft@@YAJPEAPEAUIApplicationViewPosition@@AEAUtagRECT@@@Z",
            "?GetFrameWindow@CApplicationFrameWrapper@@UEAAJPEAPEAUHWND__@@@Z",
            "?SetPresentedWindow@CApplicationFrameWrapper@@UEAAJPEAUHWND__@@@Z",
            "?SetApplicationId@CApplicationFrameWrapper@@UEAAJPEBGH@Z",
            "?FitToWorkArea@CApplicationFrameWrapper@@UEAAJXZ",
            "?GetFrameHwnd@UwpWindowWrapperBase@@UEAAPEAUHWND__@@XZ",
            "?CApplicationFrameService_CreateInstance@@YAJPEAUIImmersiveApplicationManagerInternal@@AEBU_GUID@@PEAPEAX@Z",
            "?CompleteInitialization@CApplicationFrameService@@UEAAJPEAUIServiceProvider@@@Z",
            "?EnsureFramePool@CApplicationFrameService@@UEAAJXZ",
            "?GetFrame@CApplicationFrameService@@UEAAJPEBGKPEAPEAUIApplicationFrameProxy@@@Z",
            "?g_pIAMGlobals@CApplicationManagerUtility@@3PEAVCIAMGlobals@@EA",
            "__imp_CreateWindowInBand",
            "__imp_load_CreateWindowInBand",
            "??_7UwpWindowEventDispatcher@@6B@",
            "??_7UwpWindowEventDispatcher@@6BIWeakReferenceSource@@@",
            "??_7UwpWindowEventDispatcher@@6BINtUserViewWrapperCollection@@@",
            "??_7UwpWindowWrapper@@6B?$ChainInterfaces@UIUwpWindowWrapperInternal@@UIWindowWrapper@ViewManagerInterop@Shell@Internal@Windows@@VNil@Details@WRL@Microsoft@@V789Microsoft@@V789Microsoft@@V789Microsoft@@V789Microsoft@@V789Microsoft@@V789Microsoft@@V789Microsoft@@@WRL@Microsoft@@@",
        ];
        for symbol in twinui_symbols {
            assert_ne!(
                address("twinui.pcshell.dll", symbol).unwrap(),
                0,
                "{symbol}"
            );
        }

        let _service_provider = unsafe {
            libloading::Library::new(system32.join("Windows.ImmersiveShell.ServiceProvider.dll"))
        }
        .unwrap();
        assert_ne!(
            address(
                "Windows.ImmersiveShell.ServiceProvider.dll",
                "?Start@CImmersiveShellController@@UEAAJXZ",
            )
            .unwrap(),
            0
        );
    }
}
