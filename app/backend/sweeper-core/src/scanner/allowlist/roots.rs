use super::*;

pub(in super::super) fn resolve_needed_known_folder_roots(
    targets: &[TargetDefinition],
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> BTreeMap<KnownFolder, Result<CanonicalPath, String>> {
    let needed_kfs = needed_known_folders(targets, elevation_target_root_overrides);
    let mut kf_roots = BTreeMap::new();
    for kf in needed_kfs {
        kf_roots.insert(kf, resolve_validated_known_folder_root(kf));
    }
    kf_roots
}

fn needed_known_folders(
    targets: &[TargetDefinition],
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> HashSet<KnownFolder> {
    targets
        .iter()
        .filter(|t| !is_overridden_elevation_target(t, elevation_target_root_overrides))
        .map(|t| t.known_folder)
        .collect()
}

pub(in super::super) fn is_overridden_elevation_target(
    target: &TargetDefinition,
    elevation_target_root_overrides: &BTreeMap<String, String>,
) -> bool {
    target.requires_elevation && elevation_target_root_overrides.contains_key(&target.id)
}

pub(super) fn resolve_validated_known_folder_root(
    kf: KnownFolder,
) -> Result<CanonicalPath, String> {
    let canon = resolve_known_folder_canon(kf)?;
    validate_known_folder_root_policy(kf, &canon)?;
    Ok(canon)
}

fn resolve_known_folder_canon(kf: KnownFolder) -> Result<CanonicalPath, String> {
    let id = map_known_folder(kf);
    let path = sweeper_winfs::resolve_known_folder(id).map_err(|e| e.to_string())?;
    CanonicalPath::from_existing_path(&path.to_string_lossy())
        .map(|v| v.into_canonical())
        .map_err(|e| e.to_string())
}

fn validate_known_folder_root_policy(kf: KnownFolder, canon: &CanonicalPath) -> Result<(), String> {
    match kf {
        KnownFolder::UserProfile | KnownFolder::LocalAppData | KnownFolder::RoamingAppData => {
            validate_user_scoped_known_folder_root(canon)
        }
        KnownFolder::Windows | KnownFolder::ProgramData => Ok(()),
    }
}

fn validate_user_scoped_known_folder_root(canon: &CanonicalPath) -> Result<(), String> {
    if canon.segment_count() < 3 {
        return Err("known folder root depth < 3".to_string());
    }
    if is_protected_or_container(canon) {
        return Err("known folder root violates protected path policy".to_string());
    }
    if path_has_link_point_ancestor(canon)? {
        return Err("known folder root traverses a link point".to_string());
    }
    Ok(())
}

fn is_protected_or_container(canon: &CanonicalPath) -> bool {
    if crate::policy::is_subtree_protected(canon) {
        return true;
    }
    if crate::policy::is_ancestor_of_subtree_protected(canon) {
        return true;
    }
    crate::policy::is_protected_container_root(canon)
}

pub(super) fn map_known_folder(kf: KnownFolder) -> KnownFolderId {
    match kf {
        KnownFolder::LocalAppData => KnownFolderId::LocalAppData,
        KnownFolder::RoamingAppData => KnownFolderId::RoamingAppData,
        KnownFolder::UserProfile => KnownFolderId::UserProfile,
        KnownFolder::Windows => KnownFolderId::Windows,
        KnownFolder::ProgramData => KnownFolderId::ProgramData,
    }
}

pub(in crate::scanner) fn path_has_link_point_ancestor(
    path: &CanonicalPath,
) -> Result<bool, String> {
    let mut cur = PathBuf::from(format!(r"{}\", path.drive_prefix()));
    for seg in path.segments() {
        cur.push(seg);
        if is_link_point_prefix(&cur)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn is_link_point_prefix(cur: &Path) -> Result<bool, String> {
    match sweeper_winfs::open_for_inspection(cur)
        .and_then(|h| sweeper_winfs::inspect_reparse_point(&h))
    {
        Ok(ReparseTagClass::LinkPoint) => Ok(true),
        Ok(_) => Ok(false),
        Err(WinfsError::Win32 { code: 2 | 3, .. }) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
