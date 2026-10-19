//! APK inspection before installation: package identity, SDK levels, permissions, native ABIs
//! and a SHA-256 digest. Parses the binary AndroidManifest.xml (AXML) directly, no aapt needed.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::tr;

#[derive(Debug)]
pub enum ApkError {
    Io(std::io::Error),
    NotAnArchive(String),
    MissingManifest,
    BadManifest(String),
}

impl std::fmt::Display for ApkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Io(e) => tr!("impossibile leggere il file: {e}", "cannot read the file: {e}"),
            Self::NotAnArchive(e) => tr!("il file non è un archivio APK valido: {e}", "the file is not a valid APK archive: {e}"),
            Self::MissingManifest => tr!(
                "AndroidManifest.xml mancante: non è un APK installabile (bundle .apks/.xapk non supportati)",
                "AndroidManifest.xml missing: not an installable APK (.apks/.xapk bundles are not supported)"
            ),
            Self::BadManifest(e) => tr!("AndroidManifest.xml non valido: {e}", "invalid AndroidManifest.xml: {e}"),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for ApkError {}

impl From<std::io::Error> for ApkError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbiCompat {
    /// No native code: runs on any ABI.
    Universal,
    /// Ships native code for the host ABI.
    Native,
    /// Only ARM native code on an x86 host: needs libhoudini/libndk translation.
    NeedsTranslation,
    /// Native code for no ABI Android-x86 can run.
    Incompatible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApkInfo {
    pub path: String,
    pub file_size: u64,
    pub sha256: String,
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub label: Option<String>,
    pub permissions: Vec<String>,
    pub native_abis: Vec<String>,
    pub abi_compat: AbiCompat,
}

pub fn inspect_apk(path: &Path, host_abi: Option<&str>) -> Result<ApkInfo, ApkError> {
    let mut file = std::fs::File::open(path)?;
    let file_size = file.metadata()?.len();

    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let sha256 = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();

    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| ApkError::NotAnArchive(e.to_string()))?;

    let mut abis: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index_raw(i) {
            let name = entry.name();
            if let Some(rest) = name.strip_prefix("lib/") {
                if let Some((abi, file)) = rest.split_once('/') {
                    if file.ends_with(".so") && !abis.iter().any(|a| a == abi) {
                        abis.push(abi.to_string());
                    }
                }
            }
        }
    }
    abis.sort();

    let manifest = {
        let mut entry = archive.by_name("AndroidManifest.xml").map_err(|_| ApkError::MissingManifest)?;
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        parse_manifest(&data)?
    };

    Ok(ApkInfo {
        path: path.to_string_lossy().into_owned(),
        file_size,
        sha256,
        package: manifest.package.ok_or_else(|| ApkError::BadManifest(tr!("attributo package mancante", "package attribute missing")))?,
        version_name: manifest.version_name,
        version_code: manifest.version_code,
        min_sdk: manifest.min_sdk,
        target_sdk: manifest.target_sdk,
        label: manifest.label,
        permissions: manifest.permissions,
        abi_compat: abi_compat(&abis, host_abi),
        native_abis: abis,
    })
}

pub fn abi_compat(abis: &[String], host_abi: Option<&str>) -> AbiCompat {
    if abis.is_empty() {
        return AbiCompat::Universal;
    }
    let has = |a: &str| abis.iter().any(|x| x == a);
    match host_abi {
        Some("x86_64") if has("x86_64") || has("x86") => AbiCompat::Native,
        Some("x86_64") | Some("x86") if has("arm64-v8a") || has("armeabi-v7a") || has("armeabi") => AbiCompat::NeedsTranslation,
        Some("x86") if has("x86") => AbiCompat::Native,
        Some("arm64-v8a") if has("arm64-v8a") || has("armeabi-v7a") => AbiCompat::Native,
        Some(h) if has(h) => AbiCompat::Native,
        _ => AbiCompat::Incompatible,
    }
}

#[derive(Debug, Default, Clone)]
pub struct Manifest {
    pub package: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    pub label: Option<String>,
    pub permissions: Vec<String>,
}

const RES_XML_TYPE: u16 = 0x0003;
const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_XML_RESOURCE_MAP_TYPE: u16 = 0x0180;
const RES_XML_START_ELEMENT_TYPE: u16 = 0x0102;

const ATTR_LABEL: u32 = 0x0101_0001;
const ATTR_NAME: u32 = 0x0101_0003;
const ATTR_MIN_SDK: u32 = 0x0101_020c;
const ATTR_VERSION_CODE: u32 = 0x0101_021b;
const ATTR_VERSION_NAME: u32 = 0x0101_021c;
const ATTR_TARGET_SDK: u32 = 0x0101_0270;

const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn bad(msg: String) -> ApkError {
    ApkError::BadManifest(msg)
}

pub fn parse_manifest(data: &[u8]) -> Result<Manifest, ApkError> {
    if u16_at(data, 0) != Some(RES_XML_TYPE) {
        return Err(bad(tr!("intestazione AXML assente (manifest testuale o cifrato?)", "no AXML header (text or encrypted manifest?)")));
    }
    let mut off = u16_at(data, 2).ok_or_else(|| bad(tr!("troncato", "truncated")))? as usize;
    let mut strings: Vec<String> = Vec::new();
    let mut res_ids: Vec<u32> = Vec::new();
    let mut m = Manifest::default();

    while off + 8 <= data.len() {
        let ty = u16_at(data, off).ok_or_else(|| bad(tr!("troncato", "truncated")))?;
        let header_size = u16_at(data, off + 2).ok_or_else(|| bad(tr!("troncato", "truncated")))? as usize;
        let size = u32_at(data, off + 4).ok_or_else(|| bad(tr!("troncato", "truncated")))? as usize;
        if size < 8 || off + size > data.len() {
            return Err(bad(tr!("dimensione di un blocco non valida", "invalid chunk size")));
        }
        let chunk = &data[off..off + size];
        match ty {
            RES_STRING_POOL_TYPE => strings = parse_string_pool(chunk)?,
            RES_XML_RESOURCE_MAP_TYPE => {
                res_ids = (header_size..size).step_by(4).filter_map(|o| u32_at(chunk, o)).collect();
            }
            RES_XML_START_ELEMENT_TYPE => read_element(chunk, header_size, &strings, &res_ids, &mut m)?,
            _ => {}
        }
        off += size;
    }
    Ok(m)
}

fn read_element(chunk: &[u8], header_size: usize, strings: &[String], res_ids: &[u32], m: &mut Manifest) -> Result<(), ApkError> {
    let ext = header_size;
    let name_idx = u32_at(chunk, ext + 4).ok_or_else(|| bad(tr!("elemento troncato", "truncated element")))? as usize;
    let attr_start = u16_at(chunk, ext + 8).ok_or_else(|| bad(tr!("elemento troncato", "truncated element")))? as usize;
    let attr_size = u16_at(chunk, ext + 10).ok_or_else(|| bad(tr!("elemento troncato", "truncated element")))? as usize;
    let attr_count = u16_at(chunk, ext + 12).ok_or_else(|| bad(tr!("elemento troncato", "truncated element")))? as usize;
    let element = strings.get(name_idx).map(String::as_str).unwrap_or("");
    if attr_size < 20 {
        return Err(bad(tr!("dimensione attributo non valida", "invalid attribute size")));
    }

    for i in 0..attr_count {
        let a = ext + attr_start + i * attr_size;
        let (Some(aname), Some(raw), Some(dtype), Some(value)) =
            (u32_at(chunk, a + 4), u32_at(chunk, a + 8), chunk.get(a + 15).copied(), u32_at(chunk, a + 16))
        else {
            return Err(bad(tr!("attributo troncato", "truncated attribute")));
        };
        let aname = aname as usize;
        let res_id = res_ids.get(aname).copied().unwrap_or(0);
        let attr_name = strings.get(aname).map(String::as_str).unwrap_or("");
        let as_string = || match dtype {
            TYPE_STRING => strings.get(value as usize).cloned(),
            _ if raw != u32::MAX => strings.get(raw as usize).cloned(),
            _ => None,
        };
        let as_int = || matches!(dtype, TYPE_INT_DEC | TYPE_INT_HEX).then_some(value as u64);
        let is = |id: u32, name: &str| res_id == id || (res_id == 0 && attr_name == name);

        match element {
            "manifest" if attr_name == "package" && res_id == 0 => m.package = as_string(),
            "manifest" if is(ATTR_VERSION_CODE, "versionCode") => m.version_code = as_int(),
            "manifest" if is(ATTR_VERSION_NAME, "versionName") => m.version_name = as_string(),
            "uses-sdk" if is(ATTR_MIN_SDK, "minSdkVersion") => {
                m.min_sdk = as_int().map(|v| v as u32).or_else(|| as_string().and_then(|s| s.parse().ok()))
            }
            "uses-sdk" if is(ATTR_TARGET_SDK, "targetSdkVersion") => {
                m.target_sdk = as_int().map(|v| v as u32).or_else(|| as_string().and_then(|s| s.parse().ok()))
            }
            "uses-permission" if is(ATTR_NAME, "name") => {
                if let Some(p) = as_string() {
                    if !m.permissions.contains(&p) {
                        m.permissions.push(p);
                    }
                }
            }
            "application" if is(ATTR_LABEL, "label") => m.label = as_string(),
            _ => {}
        }
    }
    Ok(())
}

fn parse_string_pool(chunk: &[u8]) -> Result<Vec<String>, ApkError> {
    let header_size = u16_at(chunk, 2).ok_or_else(|| bad(tr!("string pool troncato", "truncated string pool")))? as usize;
    let count = u32_at(chunk, 8).ok_or_else(|| bad(tr!("string pool troncato", "truncated string pool")))? as usize;
    let flags = u32_at(chunk, 16).ok_or_else(|| bad(tr!("string pool troncato", "truncated string pool")))?;
    let strings_start = u32_at(chunk, 20).ok_or_else(|| bad(tr!("string pool troncato", "truncated string pool")))? as usize;
    let utf8 = flags & 0x100 != 0;
    if count > chunk.len() / 4 {
        return Err(bad(tr!("numero di stringhe non plausibile", "implausible string count")));
    }

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let rel = u32_at(chunk, header_size + i * 4).ok_or_else(|| bad(tr!("offset stringa mancante", "missing string offset")))? as usize;
        let mut p = strings_start + rel;
        let s = if utf8 {
            let len8 = |p: &mut usize| -> Option<usize> {
                let b0 = *chunk.get(*p)? as usize;
                if b0 & 0x80 != 0 {
                    let b1 = *chunk.get(*p + 1)? as usize;
                    *p += 2;
                    Some(((b0 & 0x7f) << 8) | b1)
                } else {
                    *p += 1;
                    Some(b0)
                }
            };
            let _utf16_len = len8(&mut p).ok_or_else(|| bad(tr!("stringa troncata", "truncated string")))?;
            let n = len8(&mut p).ok_or_else(|| bad(tr!("stringa troncata", "truncated string")))?;
            String::from_utf8_lossy(chunk.get(p..p + n).ok_or_else(|| bad(tr!("stringa troncata", "truncated string")))?).into_owned()
        } else {
            let mut n = u16_at(chunk, p).ok_or_else(|| bad(tr!("stringa troncata", "truncated string")))? as usize;
            p += 2;
            if n & 0x8000 != 0 {
                n = ((n & 0x7fff) << 16) | u16_at(chunk, p).ok_or_else(|| bad(tr!("stringa troncata", "truncated string")))? as usize;
                p += 2;
            }
            let units: Vec<u16> = (0..n).map(|k| u16_at(chunk, p + k * 2)).collect::<Option<_>>().ok_or_else(|| bad(tr!("stringa troncata", "truncated string")))?;
            String::from_utf16_lossy(&units)
        };
        out.push(s);
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Builds real binary AXML manifests and APK archives for tests.

    use std::io::Write;

    pub struct Attr {
        pub name: &'static str,
        pub value: AttrValue,
    }

    pub enum AttrValue {
        Str(&'static str),
        Int(u32),
    }

    /// Attribute names that carry an android: resource id; they must come first in the pool.
    const RES_NAMES: &[(&str, u32)] = &[
        ("versionCode", 0x0101021b),
        ("versionName", 0x0101021c),
        ("minSdkVersion", 0x0101020c),
        ("targetSdkVersion", 0x01010270),
        ("name", 0x01010003),
        ("label", 0x01010001),
    ];

    pub fn build_axml(elements: &[(&'static str, Vec<Attr>)], utf16: bool) -> Vec<u8> {
        let mut strings: Vec<String> = RES_NAMES.iter().map(|(n, _)| n.to_string()).collect();
        let idx = |s: &str, strings: &mut Vec<String>| -> u32 {
            if let Some(i) = strings.iter().position(|x| x == s) {
                return i as u32;
            }
            strings.push(s.to_string());
            (strings.len() - 1) as u32
        };
        let mut body = Vec::new();
        for (el, attrs) in elements {
            let name = idx(el, &mut strings);
            let mut chunk = Vec::new();
            chunk.extend(0x0102u16.to_le_bytes());
            chunk.extend(16u16.to_le_bytes());
            chunk.extend(0u32.to_le_bytes()); // size, patched below
            chunk.extend(1u32.to_le_bytes()); // line
            chunk.extend(u32::MAX.to_le_bytes()); // comment
            chunk.extend(u32::MAX.to_le_bytes()); // ns
            chunk.extend(name.to_le_bytes());
            chunk.extend(20u16.to_le_bytes()); // attributeStart
            chunk.extend(20u16.to_le_bytes()); // attributeSize
            chunk.extend((attrs.len() as u16).to_le_bytes());
            chunk.extend([0u8; 6]); // id/class/style index
            for a in attrs {
                let an = idx(a.name, &mut strings);
                let (raw, dtype, data) = match a.value {
                    AttrValue::Str(s) => {
                        let si = idx(s, &mut strings);
                        (si, 0x03u8, si)
                    }
                    AttrValue::Int(v) => (u32::MAX, 0x10u8, v),
                };
                chunk.extend(u32::MAX.to_le_bytes());
                chunk.extend(an.to_le_bytes());
                chunk.extend(raw.to_le_bytes());
                chunk.extend(8u16.to_le_bytes());
                chunk.push(0);
                chunk.push(dtype);
                chunk.extend(data.to_le_bytes());
            }
            let size = chunk.len() as u32;
            chunk[4..8].copy_from_slice(&size.to_le_bytes());
            body.extend(chunk);
        }

        // String pool.
        let mut data = Vec::new();
        let mut offsets = Vec::new();
        for s in &strings {
            offsets.push(data.len() as u32);
            if utf16 {
                let units: Vec<u16> = s.encode_utf16().collect();
                data.extend((units.len() as u16).to_le_bytes());
                for u in units {
                    data.extend(u.to_le_bytes());
                }
                data.extend(0u16.to_le_bytes());
            } else {
                data.push(s.chars().count() as u8);
                data.push(s.len() as u8);
                data.extend(s.as_bytes());
                data.push(0);
            }
        }
        while data.len() % 4 != 0 {
            data.push(0);
        }
        let header = 28usize;
        let strings_start = header + offsets.len() * 4;
        let mut pool = Vec::new();
        pool.extend(0x0001u16.to_le_bytes());
        pool.extend((header as u16).to_le_bytes());
        pool.extend(((strings_start + data.len()) as u32).to_le_bytes());
        pool.extend((strings.len() as u32).to_le_bytes());
        pool.extend(0u32.to_le_bytes());
        pool.extend((if utf16 { 0u32 } else { 0x100u32 }).to_le_bytes());
        pool.extend((strings_start as u32).to_le_bytes());
        pool.extend(0u32.to_le_bytes());
        for o in offsets {
            pool.extend(o.to_le_bytes());
        }
        pool.extend(data);

        let mut resmap = Vec::new();
        resmap.extend(0x0180u16.to_le_bytes());
        resmap.extend(8u16.to_le_bytes());
        resmap.extend(((8 + RES_NAMES.len() * 4) as u32).to_le_bytes());
        for (_, id) in RES_NAMES {
            resmap.extend(id.to_le_bytes());
        }

        let mut out = Vec::new();
        out.extend(0x0003u16.to_le_bytes());
        out.extend(8u16.to_le_bytes());
        let total = 8 + pool.len() + resmap.len() + body.len();
        out.extend((total as u32).to_le_bytes());
        out.extend(pool);
        out.extend(resmap);
        out.extend(body);
        out
    }

    pub fn demo_manifest(package: &'static str) -> Vec<u8> {
        build_axml(
            &[
                (
                    "manifest",
                    vec![
                        Attr { name: "versionCode", value: AttrValue::Int(42) },
                        Attr { name: "versionName", value: AttrValue::Str("1.2.3") },
                        Attr { name: "package", value: AttrValue::Str(package) },
                    ],
                ),
                (
                    "uses-sdk",
                    vec![Attr { name: "minSdkVersion", value: AttrValue::Int(24) }, Attr { name: "targetSdkVersion", value: AttrValue::Int(34) }],
                ),
                ("uses-permission", vec![Attr { name: "name", value: AttrValue::Str("android.permission.INTERNET") }]),
                ("application", vec![Attr { name: "label", value: AttrValue::Str("Demo App") }]),
            ],
            false,
        )
    }

    pub fn write_apk(path: &std::path::Path, manifest: &[u8], libs: &[&str]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("AndroidManifest.xml", opts).unwrap();
        zip.write_all(manifest).unwrap();
        zip.start_file("classes.dex", opts).unwrap();
        zip.write_all(b"dex\n035\0").unwrap();
        for lib in libs {
            zip.start_file(*lib, opts).unwrap();
            zip.write_all(b"\x7fELF").unwrap();
        }
        zip.finish().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn parses_utf8_manifest() {
        let m = parse_manifest(&demo_manifest("com.example.demo")).unwrap();
        assert_eq!(m.package.as_deref(), Some("com.example.demo"));
        assert_eq!(m.version_code, Some(42));
        assert_eq!(m.version_name.as_deref(), Some("1.2.3"));
        assert_eq!(m.min_sdk, Some(24));
        assert_eq!(m.target_sdk, Some(34));
        assert_eq!(m.permissions, vec!["android.permission.INTERNET"]);
        assert_eq!(m.label.as_deref(), Some("Demo App"));
    }

    #[test]
    fn parses_utf16_manifest() {
        let axml = build_axml(&[("manifest", vec![Attr { name: "package", value: AttrValue::Str("org.fdroid.fdroid") }])], true);
        assert_eq!(parse_manifest(&axml).unwrap().package.as_deref(), Some("org.fdroid.fdroid"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_manifest(b"<manifest/>").is_err());
        let mut axml = demo_manifest("a.b");
        axml.truncate(40);
        assert!(parse_manifest(&axml).is_err());
    }

    #[test]
    fn inspects_full_apk_and_abis() {
        let dir = tempfile::tempdir().unwrap();
        let apk = dir.path().join("demo.apk");
        write_apk(&apk, &demo_manifest("com.example.demo"), &["lib/arm64-v8a/libgame.so", "lib/armeabi-v7a/libgame.so"]);
        let info = inspect_apk(&apk, Some("x86_64")).unwrap();
        assert_eq!(info.package, "com.example.demo");
        assert_eq!(info.native_abis, vec!["arm64-v8a", "armeabi-v7a"]);
        assert_eq!(info.abi_compat, AbiCompat::NeedsTranslation);
        assert_eq!(info.sha256.len(), 64);
        assert!(info.file_size > 0);
    }

    #[test]
    fn non_zip_and_missing_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.apk");
        std::fs::write(&p, b"not a zip").unwrap();
        assert!(matches!(inspect_apk(&p, None), Err(ApkError::NotAnArchive(_))));

        let file = std::fs::File::create(&p).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.finish().unwrap();
        assert!(matches!(inspect_apk(&p, None), Err(ApkError::MissingManifest)));
    }

    #[test]
    fn abi_rules() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(abi_compat(&[], Some("x86_64")), AbiCompat::Universal);
        assert_eq!(abi_compat(&v(&["x86_64", "arm64-v8a"]), Some("x86_64")), AbiCompat::Native);
        assert_eq!(abi_compat(&v(&["mips"]), Some("x86_64")), AbiCompat::Incompatible);
        assert_eq!(abi_compat(&v(&["armeabi-v7a"]), Some("arm64-v8a")), AbiCompat::Native);
    }
}
