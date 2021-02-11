use color_eyre::eyre::{eyre, ContextCompat, Result, WrapErr};
use relative_path::{RelativePath, RelativePathBuf};
use serde_derive::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn update(
    dev_dir: &Path,
    testcase_root_dir: &Path,
    testcase_relative_paths: &[String],
    verbose: bool,
) -> Result<()> {
    assert!(testcase_root_dir.is_absolute());

    check_svn_available()?;

    let (branch_url, dev_revision) = get_dev_branch_and_revision(&dev_dir, verbose)?;
    let next_dev_revision = get_next_revision(&branch_url, dev_revision, verbose)?;
    let testcase_relative_paths: Vec<_> =
        testcase_relative_paths.iter().map(PathBuf::from).collect();
    update_revision(
        &branch_url,
        next_dev_revision,
        &testcase_root_dir,
        &testcase_relative_paths,
        verbose,
    )
}

fn update_revision(
    branch_url: &str,
    next_dev_revision: Revision,
    testcase_root_dir: &Path,
    testcase_relative_paths: &[PathBuf],
    verbose: bool,
) -> Result<()> {
    assert!(testcase_root_dir.is_absolute());

    check_svn_available()?;

    if verbose {
        println!("Selected branch: {}", branch_url);
        println!("Selected revision: {}", next_dev_revision);
    }

    let testcases_revision = detect_testcases_revision(&branch_url, next_dev_revision)?;

    let mut wcs = vec![];
    for test_dir in itertools::sorted(testcase_relative_paths) {
        wcs.append(&mut svn_find_workingcopies(&testcase_root_dir, &test_dir)?);
    }
    switch_workingcopies(
        &wcs,
        &testcase_root_dir,
        &branch_url,
        testcases_revision,
        verbose,
    )?;
    Ok(())
}

pub fn checkout(
    dev_dir: &Path,
    testcase_root_dir: &Path,
    testcase_relative_paths: &[String],
    force_conversion: bool,
    minimal: bool,
    verbose: bool,
) -> Result<()> {
    assert!(testcase_root_dir.is_absolute());

    check_svn_available()?;

    let (branch_url, dev_revision) = get_dev_branch_and_revision(&dev_dir, verbose)?;
    let next_dev_revision = get_next_revision(&branch_url, dev_revision, verbose)?;
    checkout_revision(
        &branch_url,
        next_dev_revision,
        &testcase_root_dir,
        &testcase_relative_paths,
        force_conversion,
        minimal,
        verbose,
    )
}

pub fn checkout_revision(
    branch_url: &str,
    next_dev_revision: Revision,
    testcase_root_dir: &Path,
    testcase_relative_paths: &[String],
    force_conversion: bool,
    minimal: bool,
    verbose: bool,
) -> Result<()> {
    assert!(testcase_root_dir.is_absolute());

    check_svn_available()?;

    if verbose {
        println!("Selected branch: {}", branch_url);
        println!("Selected revision: {}", next_dev_revision);
    }

    let testcases_revision = detect_testcases_revision(&branch_url, next_dev_revision)?;

    let mut depth = svn_depth(&testcase_root_dir, Path::new("."));

    let testcase_relative_paths: Vec<PathBuf> =
        testcase_relative_paths.iter().map(PathBuf::from).collect();

    match &depth {
        None => {
            create_checkout_and_convert(
                &testcase_root_dir,
                &branch_url,
                testcases_revision,
                force_conversion,
                verbose,
            )
            .wrap_err("Failed to create checkout")?;

            depth = svn_depth(&testcase_root_dir, Path::new("."));
            if minimal {
                remove_unneeded_testcases(&testcase_root_dir, &testcase_relative_paths, verbose)?;
            }
        }
        Some(_) => {
            if minimal {
                // remove before switch to avoid unneded large switches
                remove_unneeded_testcases(&testcase_root_dir, &testcase_relative_paths, verbose)?;
            }
            switch_workingcopies(
                &[testcase_root_dir.to_owned()],
                &testcase_root_dir,
                &branch_url,
                testcases_revision,
                verbose,
            )?;
        }
    }

    if depth.as_deref() == Some("empty") {
        create_missing_testcases(
            &testcase_root_dir,
            &testcase_relative_paths,
            testcases_revision,
            verbose,
        )?;
    }

    Ok(())
}

/// Check if dev is up to date. If not find the last revision before future dev commits.
fn detect_testcases_revision(branch_url: &str, next_dev_revision: Revision) -> Result<Revision> {
    let mut testcases_revision = Revision::Head;
    if let Revision::Revision(rev) = next_dev_revision {
        // Only change HEAD, if we really have testcases commits after the guessed revision.
        // This makes the console output a bit nicer.
        let later_test_logs = log(
            &(branch_url.to_string() + "/testcases"),
            next_dev_revision,
            Revision::Head,
            /*limit=*/ Some(1),
        )?
        .logentry;
        if !later_test_logs.is_empty() {
            testcases_revision = Revision::Revision(rev - 1);
            println!(concat!(
                "Your dev folder is not at the latest revision. The guessed testcases ",
                "revision will be wrong, if you committed your testcase changes before ",
                "your dev changes."
            ),)
        }
    }
    Ok(testcases_revision)
}

fn create_checkout_and_convert(
    testcase_root_dir: &Path,
    branch_url: &str,
    revision: Revision,
    force_conversion: bool,
    verbose: bool,
) -> Result<()> {
    assert!(PathBuf::from(testcase_root_dir).is_absolute());

    let nested_checkouts = svn_find_workingcopies(&testcase_root_dir, Path::new("."))?;
    if !nested_checkouts.is_empty() {
        if !force_conversion {
            return Err(eyre!(concat!(
                "Aborting because of existing checkouts in testcases. ",
                "Use --force to convert them to a single sparse checkout."
            )));
        }

        if verbose {
            println!("Found nested checkouts that need conversion. ");
            println!(
                "Please don't abort or you might have to manually delete your testcases folder!"
            );
        }
        // Switching has to be done before conversion to avoid new local changes
        // when .svn index is deleted
        switch_workingcopies(
            &nested_checkouts,
            &testcase_root_dir,
            &branch_url,
            revision,
            verbose,
        )?;
    }

    for wc in &nested_checkouts {
        let status = status(&wc)?;
        let has_not_allowed_status = status.target.entry.iter().any(|t| {
            ["conflicted", "unversioned", "added", "deleted", "replaced"]
                .contains(&t.wc_status.item.as_str())
        });
        if has_not_allowed_status {
            return Err(eyre!(
                concat!(
                    "Can't proceed because of uncommitted changes in '{:?}'. ",
                    "Please solve those manually or delete the whole testcases folder."
                ),
                wc
            ));
        }
    }

    if verbose {
        println!("Creating sparse checkout {:?}", testcase_root_dir);
    }
    svn(&[
        "checkout",
        "--depth=empty",
        "--force",
        &format!("{}/testcases@{}", branch_url, revision),
        testcase_root_dir.to_str().unwrap(),
    ])?;

    if !nested_checkouts.is_empty() {
        if verbose {
            println!("Converting nested checkouts");
        }
        for wc_path in itertools::sorted(nested_checkouts) {
            let wc_relpath = svn_relpath_p(&wc_path, testcase_root_dir);
            if verbose {
                print_svn_path(&wc_relpath);
            }
            delete_svn_index(&wc_path);
            // TODO: do nested checkouts always have depth=infinity?
            svn_make_sparse(&testcase_root_dir, &PathBuf::from(wc_relpath), revision)?;
        }
    }
    Ok(())
}

fn remove_unneeded_testcases(
    testcase_root_dir: &Path,
    testcase_relative_paths: &[PathBuf],
    verbose: bool,
) -> Result<()> {
    let mut unneeded_paths = vec![];

    fn recursive_find_unneeded(
        path: &Path,
        testcase_root_dir: &Path,
        testcase_relative_paths: &[PathBuf],
        mut unneeded_paths: &mut Vec<String>,
    ) {
        let abs_path = testcase_root_dir.join(path);
        if !abs_path.exists() || path.file_name().map(|n| n == ".svn").unwrap_or(false) {
            return;
        }
        if !testcase_relative_paths
            .iter()
            .any(|p| subpath_of(p.to_str().unwrap(), path.to_str().unwrap()))
        {
            unneeded_paths.push(path.to_str().unwrap().to_string());
        } else if !testcase_relative_paths
            .iter()
            .any(|p| subpath_of(path.to_str().unwrap(), p.to_str().unwrap()))
            && abs_path.is_dir()
        {
            for f in std::fs::read_dir(abs_path).unwrap() {
                recursive_find_unneeded(
                    &path.join(f.unwrap().path().file_name().unwrap()),
                    &testcase_root_dir,
                    &testcase_relative_paths,
                    &mut unneeded_paths,
                );
            }
        }
    }

    recursive_find_unneeded(
        &Path::new("."),
        &testcase_root_dir,
        &testcase_relative_paths,
        &mut unneeded_paths,
    );

    if !unneeded_paths.is_empty() {
        if verbose {
            println!("Removing unneeded checkouts");
        }
        for path in itertools::sorted(unneeded_paths) {
            if verbose {
                print_svn_path(&path);
            }
            let status = status(&testcase_root_dir.join(&path))?;
            if !status.target.entry.is_empty() {
                println!(
                    "Cannot remove {:?}, it contains changes or unversioned files",
                    path
                );
            } else {
                svn_wd(
                    &["update", "--set-depth=exclude", &path],
                    &testcase_root_dir,
                )?;
            }
        }
    }
    Ok(())
}

fn create_missing_testcases(
    testcase_root_dir: &Path,
    testcase_relative_paths: &[PathBuf],
    revision: Revision,
    verbose: bool,
) -> Result<()> {
    let mut missing_paths = vec![];
    for test_path in testcase_relative_paths {
        if svn_depth(&test_path, testcase_root_dir).as_deref() != Some("infinity") {
            missing_paths.push(test_path);
        }
    }

    if !missing_paths.is_empty() {
        if verbose {
            println!("Downloading missing testcases");
        }
        for test_path in itertools::sorted(missing_paths) {
            if verbose {
                print_svn_path_p(&test_path);
            }
            svn_make_sparse(&testcase_root_dir, &test_path, revision)?;
        }
    }
    Ok(())
}

fn switch_workingcopies(
    wcs: &[PathBuf],
    testcases_root_path: &Path,
    branch_url: &str,
    revision: Revision,
    verbose: bool,
) -> Result<()> {
    // needs to be absolute, otherwise svn_relpath doesn't work
    assert!(testcases_root_path.is_absolute());

    if verbose {
        println!("Switching {:?} to {}", testcases_root_path, revision);
    }
    if wcs.is_empty() {
        if verbose {
            println!("  - No checkouts found");
        }
    } else {
        let mut wcs = Vec::from(wcs);
        wcs.sort();
        wcs.dedup();
        for wc in wcs {
            let wc_relpath = svn_relpath_p(&wc, &testcases_root_path);
            if verbose && wc_relpath != "." && !wc_relpath.is_empty() {
                print_svn_path(&wc_relpath);
            }
            let target_url =
                svn_resolve_relpath(&(branch_url.to_owned() + "/testcases"), &wc_relpath);
            let result = svn(&[
                "switch",
                "--accept=postpone",
                &format!("{}@{}", target_url, revision),
                &wc.to_str().unwrap(),
            ])?;
            if svn_had_conflicts(&result) {
                println!("conflict in {:?}. Please use svn to resolve it!", wc);
            }
        }
    }
    Ok(())
}

fn svn_make_sparse(root: &Path, path: &Path, revision: Revision) -> Result<()> {
    let path_list = path_to_list(path.to_str().unwrap());
    for i in 0..path_list.len() {
        let sub_path = itertools::join(&path_list[0..=i], "/");
        let sub_path = Path::new(&sub_path);
        let current_depth = svn_depth(&sub_path, root);
        let needed_depth = if i + 1 < path_list.len() {
            "empty"
        } else {
            "infinity"
        };
        let depth_order = ["exclude", "empty", "files", "immediates", "infinity"];
        // Avoid for example setting infinity to empty
        // to not delete lots of already checked out files!
        let needs_update = match current_depth {
            None => true,
            Some(current_depth) => {
                depth_order
                    .iter()
                    .position(|e| *e == current_depth)
                    .unwrap()
                    < depth_order.iter().position(|e| *e == needed_depth).unwrap()
            }
        };
        if needs_update {
            svn_wd(
                &[
                    "update",
                    &format!("--set-depth={}", needed_depth),
                    "--force",
                    "--accept=postpone",
                    "--revision",
                    &revision.to_string(),
                    &sub_path.to_str().unwrap(),
                ],
                &root,
            )?;

            // svn update silently does nothing if an url does not exist.
            // => Check if something was created locally
            let abs_path = root.join(sub_path);
            svn_depth(&abs_path, root)
                .wrap_err("Path does not exist in SVN. Did you pass the correct test id?")?;
        }
    }
    Ok(())
}

/// Traverses recursively through subdirectories, if no svn working copy found.
/// Validates working copies
fn svn_find_workingcopies(root: &Path, relpath: &Path) -> Result<Vec<PathBuf>> {
    let mut relpath_to_wcs = vec![];
    let abs_path = root.join(relpath);
    if abs_path.exists() {
        let svn_info = info(&abs_path.to_str().unwrap().replace('\\', "/"))?;
        match &svn_info.entry {
            Some(entry) => {
                if !path_endswith(&entry.url, relpath.to_str().unwrap()) {
                    println!(
                        "Ignoring unexpected subdirectories in svn url {}. Does not fit to {:?}",
                        svn_info.entry.as_ref().unwrap().url,
                        relpath
                    );
                } else {
                    relpath_to_wcs.push(PathBuf::from(
                        &svn_info.entry.as_ref().unwrap().wc_info.wc_root_path,
                    ));
                }
            }
            None => {
                // Only if the current path is not an svn checkout we search in subdirs.
                // This misses nested svn checkouts, which is possible but unlikely to happen.
                if abs_path.is_dir() {
                    for subdir in std::fs::read_dir(abs_path)? {
                        let subdir = subdir?;
                        let abs_subdir = subdir.path();
                        if abs_subdir.is_dir() {
                            relpath_to_wcs.append(&mut svn_find_workingcopies(
                                root,
                                &(relpath.join(subdir.file_name())),
                            )?);
                        }
                    }
                }
            }
        }
    }
    Ok(relpath_to_wcs)
}

fn delete_svn_index(working_copy_path: &Path) {
    let index_path = working_copy_path.join(".svn");
    if index_path.is_dir() {
        std::fs::remove_dir_all(index_path).unwrap();
    }
}

fn path_to_list(path: &str) -> Vec<String> {
    // TODO: maybe there is another way to do this with rust os functions
    // TODO: will not work for '..' in the path
    let mut list: Vec<_> = path
        .replace('\\', "/")
        .replace("%20", " ")
        .split('/')
        .filter(|t| *t != ".")
        .map(|t| t.to_string())
        .collect();
    if list.last().map(|s| s.as_str()) == Some("") {
        list.pop();
    }
    list
}

fn path_endswith(path: &str, endpath: &str) -> bool {
    let split_path = path_to_list(path);
    let split_endpath = path_to_list(endpath);
    if split_endpath.is_empty() {
        return true;
    }
    if split_path.len() < split_endpath.len() {
        return false;
    }
    let r1 = split_path.iter().rev();
    let r2 = split_endpath.iter().rev();
    r1.zip(r2).all(|(t1, t2)| t1 == t2)
}

fn subpath_of(subpath: &str, path: &str) -> bool {
    let relpath = RelativePath::new(path).relative(RelativePath::new(subpath));
    let relpath = relpath.as_str();
    relpath == "." || !relpath.starts_with("..")
}

fn svn_relpath_p(to: &Path, from: &Path) -> String {
    let to = RelativePathBuf::from(to.to_str().unwrap().replace('\\', "/"));
    let from = RelativePathBuf::from(from.to_str().unwrap().replace('\\', "/"));
    from.relative(to).as_str().to_owned()
}

/// E.g. /A/B/C, ../../D becomes /A/D
fn svn_resolve_relpath(url: &str, relpath: &str) -> String {
    let mut url_list = path_to_list(url);
    let relpath_list = path_to_list(relpath);
    for subdir in relpath_list.into_iter() {
        if subdir == "." {
            continue;
        } else if subdir == ".." {
            if url_list.is_empty() {
                panic!("Cannot resolve svn path {}/{}", url, relpath);
            }
            url_list.pop();
        } else {
            url_list.push(subdir);
        }
    }
    itertools::join(url_list, "/")
}

/// Returns svn branch url and revision of local dev working copy
pub fn get_dev_branch_and_revision(dev_dir: &Path, verbose: bool) -> Result<(String, u32)> {
    if verbose {
        println!("Checking {:?}", dev_dir);
    }

    let dev_info = info(&dev_dir.to_str().unwrap())?;
    let dev_info = dev_info.entry.unwrap();
    if !dev_info.relative_url.ends_with("/dev") {
        return Err(eyre!(
            "Invalid dev svn url: {}\nwhile checking {:?}",
            dev_info.url,
            dev_dir
        ));
    }
    let branch_url = &dev_info.url[0..dev_info.url.len() - 4];
    let relative_branch_url = &dev_info.relative_url[1..dev_info.relative_url.len() - 4]; // strip also ^ from start
    if verbose {
        println!("  - Branch is {}", relative_branch_url);
    }

    // By using svnversion to get the dev revision, we get the range of revisions spread
    // over the working copy (often happens when the user commits in sub directories of dev).
    // If there are no commits after the last local revision, we assume we are up to date.
    // But theoretically, someone can update a sub directory back after committing, while
    // another sub directory already has a larger revision. Then the working copy is NOT up to date.
    // TODO: Let's accept the error in this unlikely case for now, as checking all relevant
    // sub directories with svn info can be costly. Also checking for each changed path of
    // commits can be costly.
    let dev_revision = svn_revision(dev_dir)?;

    Ok((branch_url.to_owned(), dev_revision))
}

/// Returns next dev revision
fn get_next_revision(branch_url: &str, dev_revision: u32, verbose: bool) -> Result<Revision> {
    if verbose {
        println!("Checking {}", branch_url);
    }

    let dev_logs = log(
        &(branch_url.to_owned() + "/dev"),
        Revision::Revision(dev_revision + 1),
        Revision::Head,
        None,
    )?
    .logentry;
    let next_dev_revision = if dev_logs.is_empty() {
        Revision::Head
    } else {
        Revision::Revision(dev_logs.iter().map(|e| e.revision).min().unwrap())
    };

    if verbose {
        if next_dev_revision == Revision::Head {
            println!("  - Dev revision is {}", dev_revision);
        } else {
            println!("  - Dev revision is {} (NOT at HEAD)", dev_revision);
        }
    }

    Ok(next_dev_revision)
}

fn svn_depth(local_path: &Path, _cwd: &Path) -> Option<String> {
    Some(
        info(&local_path.to_str().unwrap())
            .ok()?
            .entry?
            .wc_info
            .depth,
    )
}

fn svn_revision(local_path: &Path) -> Result<u32> {
    let revisions = svn_revisions(local_path);
    revisions.map(|rs| *rs.last().unwrap())
}

fn svn_revisions(local_path: &Path) -> Result<Vec<u32>> {
    let output = Command::new("svnversion")
        .arg(&local_path)
        .output()
        .unwrap();
    let output = std::str::from_utf8(&output.stdout).unwrap();
    let mut revs = output.trim_matches(|c: char| !c.is_numeric()).split(':');
    let r1 = revs.next().map(|r| r.parse());
    let r2 = revs.next().map(|r| r.parse());
    match (r1, r2) {
        (Some(Ok(r1)), None) => Ok(vec![r1]),
        (Some(Ok(r1)), Some(Ok(r2))) => Ok(vec![r1, r2]),
        _ => Err(eyre!("Failed to parse output of 'svnversion'.")),
    }
}

fn check_svn_available() -> Result<()> {
    let output = Command::new("svn")
        .arg("--version")
        .output()
        .wrap_err(concat!(
            "Could not find svn. Please make sure you installed an svn command line client ",
            "and put it into the system searh path. For Windows for example install Tortoise SVN ",
            "and make sure 'command line client tools' are selected."
        ))?;
    let output = std::str::from_utf8(&output.stdout).unwrap();

    let re = regex::Regex::new(r"version (\d+)\.(\d+)").unwrap();
    match re.captures(output) {
        Some(cap) => {
            let (major, minor) = (cap[1].parse().unwrap(), cap[2].parse().unwrap());
            if (major, minor) >= (1, 6) {
                Ok(())
            } else {
                Err(eyre!(
                    "Found svn version {}.{}. Please install a version of at least 1.6.",
                    major,
                    minor
                ))
            }
        }
        None => {
            Err(eyre!(
                "Could not validate version of svn. Please make sure a recent svn with at least version 1.6 is installed."
            ))
        }
    }
}

fn svn_had_conflicts(svn_output: &str) -> bool {
    for line in svn_output.lines() {
        if line.starts_with("C ") {
            return true;
        }
    }
    false
}

fn print_svn_path(mut path: &str) {
    if path.starts_with("./") {
        path = &path[2..];
    };
    if path.ends_with('/') {
        path = &path[..path.len() - 1];
    }
    println!("  - {}", path);
}

fn print_svn_path_p(path: &Path) {
    print_svn_path(&path.to_str().unwrap().replace('\\', "/"));
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Revision {
    Head,
    Revision(u32),
}
impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Revision::Head => write!(f, "HEAD"),
            Revision::Revision(r) => write!(f, "{}", r),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename = "status")]
struct Status {
    target: StatusTarget,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "entry")]
struct StatusTarget {
    #[serde(default)]
    entry: Vec<StatusEntry>,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "entry")]
struct StatusEntry {
    path: String,
    #[serde(rename = "wc-status")]
    wc_status: WcStatus,
}

#[derive(Deserialize, Debug)]
struct WcStatus {
    item: String,
}

fn status(root: &Path) -> Result<Status> {
    let output = svn(&["status", "--xml", root.to_str().unwrap()])?;
    Ok(serde_xml_rs::from_str(&output).unwrap())
}

#[derive(Deserialize, Debug)]
#[serde(rename = "info")]
struct Info {
    entry: Option<InfoEntry>,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "entry")]
struct InfoEntry {
    url: String,
    #[serde(rename = "relative-url")]
    relative_url: String,
    #[serde(rename = "wc-info")]
    wc_info: WcInfo,
}

#[derive(Deserialize, Debug)]
#[serde(rename = "wc-info")]
struct WcInfo {
    #[serde(rename = "wcroot-abspath")]
    wc_root_path: String,
    depth: String,
}

fn info(root: &str) -> Result<Info> {
    let output = Command::new("svn")
        .args(&["info", "--xml", &root])
        .output()?;
    if !output.status.success() {
        if std::str::from_utf8(&output.stderr)
            .unwrap()
            .contains("E155007:")
        {
            // E155007: [...] is not a working directory => return an empty info struct
            return Ok(Info { entry: None });
        }
        return Err(eyre!("SVN log failed: {:?}", output));
    }
    let output = svn(&["info", "--xml", root])?;
    Ok(serde_xml_rs::from_str(&output)?)
}

#[derive(Deserialize, Debug)]
#[serde(rename = "log")]
struct Log {
    #[serde(default)]
    logentry: Vec<LogEntry>,
}
#[derive(Deserialize, Debug)]
#[serde(rename = "logentry")]
struct LogEntry {
    revision: u32,
}

fn log(
    root: &str,
    revision_start: Revision,
    revision_end: Revision,
    limit: Option<u32>,
) -> Result<Log> {
    let mut cmd = Command::new("svn");
    cmd.args(&[
        "log",
        "--xml",
        "-v",
        "-r",
        &format!("{}:{}", revision_start, revision_end),
        &root,
    ]);
    if let Some(limit) = limit {
        cmd.args(&["-l", &format!("{}", limit)]);
    }
    let output = cmd.output()?;
    if !output.status.success() {
        if std::str::from_utf8(&output.stderr)
            .unwrap()
            .contains("E160006:")
        {
            // E160006: no such revision => return an empty list
            return Ok(Log { logentry: vec![] });
        }
        return Err(eyre!("SVN log failed: {:?}", output));
    }
    let output = std::str::from_utf8(&output.stdout).unwrap();
    Ok(serde_xml_rs::from_str(output)?)
}

fn svn(args: &[&str]) -> Result<String> {
    println!("svn {:?}", args);
    let output = Command::new("svn")
        .args(args)
        .output()
        .wrap_err("Failed to start SVN")?;
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    if !output.status.success() {
        return Err(eyre!("Failed to run SVN: {:?}\n{:?}", args, output));
    }
    Ok(stdout.to_owned())
}

fn svn_wd(args: &[&str], wd: &Path) -> Result<String> {
    println!("svn_args {:?}\n  wd:{:?}", args, wd);
    let output = Command::new("svn")
        .args(args)
        .current_dir(wd)
        .output()
        .wrap_err("Failed to start SVN")?;
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    println!("{:?}{:?}", stdout, stderr);
    if !output.status.success() {
        return Err(eyre!("Failed to run SVN: {:?}\n{:?}", args, output));
    }
    Ok(stdout.to_owned())
}

#[cfg(test)]
mod tests {
    use color_eyre::eyre::Result;
    use serial_test::serial;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const ROOT: &str = "test/svn with spaces";
    const BRANCH_URL: &str =
        "https://svn.moduleworks.com/ModuleWorks/trunk/testprojects/mwtest%20svn-mockup";
    const DEV_URL: &str =
        "https://svn.moduleworks.com/ModuleWorks/trunk/testprojects/mwtest%20svn-mockup/dev";
    const TEST_URL: &str =
        "https://svn.moduleworks.com/ModuleWorks/trunk/testprojects/mwtest%20svn-mockup/testcases";
    const TEST_FOLDERS: [&str; 3] = ["sample-test-dir1", "sample-test-dir2", "sample with spaces"];
    const TEST_SAMPLES: [&str; 4] = [
        "sample-test-dir1",
        "sample-test-dir2",
        "sample with spaces",
        "sample-test.txt",
    ];
    const NESTED_TEST_FILE: &str = "sample-test-dir1/sample-test.txt";

    fn setup() {
        if Path::new(ROOT).exists() {
            std::fs::remove_dir_all(Path::new(ROOT)).unwrap();
        }
        std::fs::create_dir_all(Path::new(ROOT)).unwrap();
    }

    fn checkout_empty(path: &str) {
        assert!(Command::new("svn")
            .args(&[
                "checkout",
                "--depth=empty",
                path,
                Path::new(ROOT).to_str().unwrap()
            ])
            .status()
            .unwrap()
            .success());
    }

    fn checkout(path: &str) {
        assert!(Command::new("svn")
            .args(&["checkout", path, Path::new(ROOT).to_str().unwrap()])
            .status()
            .unwrap()
            .success());
    }

    #[test]
    #[serial]
    fn svn_available() {
        setup();
        assert!(super::check_svn_available().is_ok())
    }

    #[test]
    #[serial]
    fn svn_info() {
        setup();
        checkout(DEV_URL);
        assert_eq!(
            &super::info(&Path::new(ROOT).to_str().unwrap())
                .unwrap()
                .entry
                .unwrap()
                .url,
            DEV_URL
        );
    }

    #[test]
    #[serial]
    fn svn_status() -> Result<()> {
        setup();
        checkout(DEV_URL);
        std::fs::write(Path::new(ROOT).join("test.txt"), "").unwrap();
        let entries = super::status(Path::new(ROOT))?.target.entry;
        let unversioned: Vec<_> = entries
            .iter()
            .filter(|entry| entry.wc_status.item == "unversioned")
            .collect();
        assert_eq!(1, unversioned.len());
        assert!(!unversioned[0].path.is_empty());
        Ok(())
    }

    #[test]
    #[serial]
    fn svn_depth() {
        setup();
        assert!(super::svn_depth(Path::new(ROOT), Path::new(".")).is_none());
        checkout_empty(DEV_URL);
        assert_eq!(
            super::svn_depth(Path::new(ROOT), Path::new(".")).as_deref(),
            Some("empty")
        );
        setup();
        checkout(DEV_URL);
        assert_eq!(
            super::svn_depth(Path::new(ROOT), Path::new(".")).as_deref(),
            Some("infinity")
        );
    }

    #[test]
    #[serial]
    fn svn_revision() {
        setup();
        assert!(super::svn_revision(Path::new(ROOT)).is_err());
        checkout(DEV_URL);
        assert!(super::svn_revision(Path::new(ROOT)).is_ok());
    }

    #[test]
    #[serial]
    fn svn_multiple_revisions() -> Result<()> {
        setup();
        checkout(TEST_URL);
        let revision = super::svn_revision(Path::new(ROOT))?;
        assert_eq!(super::svn_revisions(Path::new(ROOT))?, vec![revision]);

        let subfolder_path = PathBuf::from(Path::new(ROOT)).join(TEST_FOLDERS[0]);
        super::svn(&[
            "update",
            &subfolder_path.to_str().unwrap(),
            "-r",
            &format!("{}", revision - 1),
        ])?;
        assert_eq!(
            super::svn_revisions(Path::new(ROOT))?,
            vec![revision - 1, revision]
        );
        Ok(())
    }

    #[test]
    #[serial]
    fn svn_branch_info() -> Result<()> {
        setup();
        checkout(DEV_URL);

        // Rely on svn mockup repo:
        // 819767 -> dev commit
        // 819768 -> other commit
        // 819769 -> dev commit
        super::svn(&["update", Path::new(ROOT).to_str().unwrap(), "-r", "819767"])?;

        let (branch, revision) = super::get_dev_branch_and_revision(Path::new(ROOT), false)?;
        let next_revision = super::get_next_revision(&branch, revision, false)?;

        assert_eq!(branch, BRANCH_URL);
        // assertEqual(revision, 819767)
        assert_eq!(next_revision, super::Revision::Revision(819769));

        super::svn(&["update", Path::new(ROOT).to_str().unwrap()])?;

        let (branch, revision) = super::get_dev_branch_and_revision(Path::new(ROOT), false)?;
        let next_revision = super::get_next_revision(&branch, revision, false)?;
        assert_eq!(next_revision, super::Revision::Head);

        setup();
        checkout(TEST_URL);
        assert!(super::get_dev_branch_and_revision(Path::new(ROOT), false).is_err());
        //let next_revision = super::get_next_revision(&branch, revision, false)?;
        Ok(())
    }

    // TODO: test_detect_testcases_revision

    #[test]
    #[serial]
    fn delete_svn_index() {
        setup();
        checkout(DEV_URL);
        let index_path = PathBuf::from(Path::new(ROOT)).join(".svn");
        assert!(index_path.is_dir());
        super::delete_svn_index(Path::new(ROOT));
        assert!(!index_path.is_dir());
    }

    #[test]
    #[serial]
    fn svn_find_workingcopies() {
        setup();
        for f in &TEST_FOLDERS {
            super::svn(&[
                "checkout",
                &(TEST_URL.to_owned() + "/" + f),
                &(Path::new(ROOT).join(f).to_str().unwrap()),
            ])
            .unwrap();
        }

        assert_eq!(
            super::svn_find_workingcopies(Path::new(ROOT), Path::new("."))
                .unwrap()
                .len(),
            TEST_FOLDERS.len()
        );
        for f in &TEST_FOLDERS {
            let dirs = super::svn_find_workingcopies(Path::new(ROOT), Path::new(f)).unwrap();
            assert!(dirs[0].ends_with(f));
        }
        assert_eq!(
            super::svn_find_workingcopies(Path::new(ROOT), Path::new("./non-existant-subdir"))
                .unwrap(),
            Vec::<PathBuf>::new()
        );

        super::svn(&[
            "checkout",
            &(TEST_URL.to_owned() + "/" + TEST_FOLDERS[0]),
            &Path::new(ROOT).join("someOtherDirName/").to_str().unwrap(),
        ])
        .unwrap();
        super::svn_find_workingcopies(Path::new(ROOT), Path::new(".")).unwrap();
    }

    #[test]
    #[serial]
    fn svn_make_sparse() {
        setup();
        checkout_empty(TEST_URL);

        super::svn_make_sparse(
            Path::new(ROOT),
            Path::new(NESTED_TEST_FILE),
            super::Revision::Head,
        )
        .unwrap();
        assert!(PathBuf::from(Path::new(ROOT))
            .join(NESTED_TEST_FILE)
            .exists());

        assert!(super::svn_make_sparse(
            Path::new(ROOT),
            Path::new("asdasd/asdasd/asdklasjdlkasjdk"),
            super::Revision::Head
        )
        .is_err());
    }

    #[test]
    #[serial]
    fn switch_workingcopies() {
        setup();

        let root = PathBuf::from(Path::new(ROOT)).canonicalize().unwrap();

        // should not throw for no tests
        super::switch_workingcopies(&[], &root, "", super::Revision::Revision(0), false).unwrap();

        checkout(TEST_URL);
        let initial_revision = super::svn_revision(Path::new(ROOT)).unwrap();
        let new_revision = super::Revision::Revision(initial_revision - 1);
        super::switch_workingcopies(&[root.clone()], &root, BRANCH_URL, new_revision, false)
            .unwrap();
        assert_eq!(
            super::Revision::Revision(super::svn_revision(Path::new(ROOT)).unwrap()),
            new_revision
        );
    }

    #[test]
    #[serial]
    fn create_checkout_and_convert() {
        setup();

        let abs_test_dir = Path::new(ROOT).join(TEST_FOLDERS[0]);
        let test_url = TEST_URL.to_owned() + "/" + TEST_FOLDERS[0];
        super::svn(&["checkout", &test_url, &abs_test_dir.to_str().unwrap()]).unwrap();

        let revision = super::svn_revision(&abs_test_dir).unwrap();
        let root = Path::new(ROOT).canonicalize().unwrap();
        let force = true;
        super::create_checkout_and_convert(
            &root,
            BRANCH_URL,
            super::Revision::Revision(revision),
            force,
            false,
        )
        .unwrap();

        assert_eq!(super::svn_revision(Path::new(ROOT)).unwrap(), revision);
        assert!(PathBuf::from(&abs_test_dir).exists());
        assert!(!PathBuf::from(&abs_test_dir).join(".svn").exists());
        let num_files = std::fs::read_dir(abs_test_dir).unwrap().count();
        assert_ne!(num_files, 0);
    }

    #[test]
    #[serial]
    fn create_missing_testcases() {
        setup();
        checkout_empty(TEST_URL);
        let root = Path::new(ROOT).canonicalize().unwrap();
        let test_samples: Vec<_> = TEST_SAMPLES.iter().map(PathBuf::from).collect();
        super::create_missing_testcases(&root, &test_samples, super::Revision::Head, true).unwrap();
        for test in &TEST_SAMPLES {
            assert!(PathBuf::from(Path::new(ROOT)).join(test).exists());
        }
    }

    #[test]
    #[serial]
    fn test_checkout() {
        setup();
        let dev_dir = Path::new(ROOT).to_owned().join("dev");
        let test_dir = Path::new(ROOT).to_owned().join("testcases");
        let test_dir_abs = PathBuf::from(Path::new(ROOT))
            .canonicalize()
            .unwrap()
            .join("testcases");
        std::fs::create_dir(&test_dir_abs).unwrap();

        super::svn(&["checkout", DEV_URL, &dev_dir.to_str().unwrap()]).unwrap();
        let dev_revision = super::svn_revision(&dev_dir).unwrap();

        for _ in 0..2 {
            let force = true;
            let test_samples: Vec<String> = TEST_SAMPLES.iter().map(|s| s.to_string()).collect();
            super::checkout(&dev_dir, &test_dir_abs, &test_samples, force, false, false).unwrap();
            for test in &TEST_SAMPLES {
                assert!(super::svn_revision(&(test_dir.join(test))).unwrap() >= dev_revision);
            }
        }
    }

    #[test]
    #[serial]
    fn test_update() {
        setup();
        let dev_dir = Path::new(ROOT).join("dev");
        super::svn(&[
            "checkout",
            "--depth=empty",
            DEV_URL,
            &dev_dir.to_str().unwrap(),
        ])
        .unwrap();
        let dev_revision = super::svn_revision(&dev_dir).unwrap();

        let test_dir = Path::new(ROOT).join("testcases");
        let test_sample = &TEST_SAMPLES[0];
        super::svn(&[
            "checkout",
            "--depth=empty",
            &(TEST_URL.to_owned() + "/" + test_sample),
            &test_dir.join(test_sample).to_str().unwrap(),
        ])
        .unwrap();

        let test_sample_relurl = "testcases/".to_owned() + test_sample;
        super::svn_wd(
            &[
                "update",
                &test_sample_relurl,
                "--revision",
                &format!("{}", dev_revision - 1),
            ],
            Path::new(ROOT),
        )
        .unwrap();
        let test_sample_url = Path::new(ROOT).join(&test_sample_relurl);
        assert!(super::svn_revision(&test_sample_url).unwrap() < dev_revision);

        for i in 0..1 {
            let test_dir_abs = PathBuf::from(&test_dir).canonicalize().unwrap();
            super::update(&dev_dir, &test_dir_abs, &[test_sample.to_string()], false).unwrap();
            assert!(super::svn_revision(&test_sample_url).unwrap() >= dev_revision);
            if i == 0 {
                // In second pass check with a converted repository
                super::create_checkout_and_convert(
                    &test_dir_abs,
                    BRANCH_URL,
                    super::Revision::Revision(dev_revision),
                    true,
                    false,
                )
                .unwrap();
            }
        }
    }

    #[test]
    #[serial]
    fn complete_branch_checkout() {
        setup();
        checkout_empty(BRANCH_URL);
        let root = PathBuf::from(Path::new(ROOT)).canonicalize().unwrap();
        let dev_dir = root.join("dev");
        let test_dir = root.join("testcases");
        super::svn_wd(&["update", "--set-depth=empty", "dev"], Path::new(ROOT)).unwrap();
        println!("{:?}", dev_dir);
        let revision = super::svn_revision(&dev_dir).unwrap();
        super::svn_wd(
            &[
                "update",
                "--set-depth=empty",
                "-r",
                &format!("{}", revision - 1),
                "testcases",
            ],
            Path::new(ROOT),
        )
        .unwrap();

        super::update(&dev_dir, &test_dir, &[".".to_string()], false).unwrap();
        assert!(super::svn_revision(&test_dir).unwrap() >= revision);

        let test_samples: Vec<String> = TEST_SAMPLES.iter().map(|s| s.to_string()).collect();
        super::checkout(&dev_dir, &test_dir, &test_samples, true, false, false).unwrap();
        for test in &TEST_SAMPLES {
            assert!(test_dir.join(test).exists());
        }
    }

    #[test]
    #[serial]
    fn remove_unneeded_testcases() {
        setup();
        checkout_empty(TEST_URL);
        for test in &TEST_SAMPLES {
            super::svn_make_sparse(Path::new(ROOT), Path::new(test), super::Revision::Head)
                .unwrap();
        }
        let some_testfolder = &TEST_FOLDERS[0];
        let some_testfolder_path = Path::new(ROOT).join(some_testfolder);
        assert!(PathBuf::from(&some_testfolder_path).exists());
        assert!(std::fs::read_dir(&some_testfolder_path).unwrap().count() > 0);

        super::remove_unneeded_testcases(Path::new(ROOT), &[PathBuf::from(some_testfolder)], false)
            .unwrap();
        assert!(PathBuf::from(&some_testfolder_path).exists());
        assert!(std::fs::read_dir(&some_testfolder_path).unwrap().count() > 0);
        for other_sample in &TEST_SAMPLES {
            if other_sample != some_testfolder {
                assert!(!PathBuf::from(Path::new(ROOT)).join(other_sample).exists());
                // make sure depth 'excluded' is not confusing later checkouts
                super::svn_make_sparse(
                    Path::new(ROOT),
                    Path::new(other_sample),
                    super::Revision::Head,
                )
                .unwrap();
            }
        }

        // unversioned files should not be deleted
        let unversioned_file = PathBuf::from(Path::new(ROOT))
            .join(TEST_FOLDERS[1])
            .join("some-unversioned-file.txt");
        std::fs::write(&unversioned_file, "").unwrap();
        assert!(unversioned_file.exists());

        // and a warning should be emited
        //with warnings.catch_warnings(record=True) as caught_warnings:
        super::remove_unneeded_testcases(Path::new(ROOT), &[PathBuf::from(some_testfolder)], false)
            .unwrap();
        assert!(unversioned_file.exists());
        //assertNotEqual(len(caught_warnings), 0)

        std::fs::remove_file(&unversioned_file).unwrap();
        // double check if it works again
        //with warnings.catch_warnings(record=True) as caught_warnings:
        super::remove_unneeded_testcases(Path::new(ROOT), &[PathBuf::from(some_testfolder)], false)
            .unwrap();
        //assertEqual(len(caught_warnings), 0)
    }

    #[test]
    #[serial]
    fn test_logs() {
        setup();
        // Use known commits in test mockup repository:
        // 819699, 819696, 819695
        checkout_empty(BRANCH_URL);
        let some_logs = super::log(
            Path::new(ROOT).to_str().unwrap(),
            super::Revision::Revision(819695),
            super::Revision::Revision(819699),
            None,
        )
        .unwrap();
        let all_logs = super::log(
            Path::new(ROOT).to_str().unwrap(),
            super::Revision::Revision(819695),
            super::Revision::Head,
            None,
        )
        .unwrap();
        assert_eq!(some_logs.logentry.len(), 3);
        assert!(some_logs.logentry.iter().any(|le| le.revision == 819696));
        assert!(all_logs.logentry.len() > 3);
    }

    #[test]
    #[serial]
    fn test_logs_encoding() {
        setup();
        // check that script doesn't choke on weird characters in commit messages
        // use known commit: 909029
        checkout_empty(
            "https://svn.moduleworks.com/ModuleWorks/trunk/testprojects/mwtest svn-encoding",
        );
        let all_logs = super::log(
            Path::new(ROOT).to_str().unwrap(),
            super::Revision::Revision(909029),
            super::Revision::Revision(909029),
            None,
        )
        .unwrap();
        assert_eq!(all_logs.logentry.len(), 1)
    }

    #[test]
    fn path_to_list() {
        assert_eq!(super::path_to_list("/A/B"), vec!["", "A", "B"]);
    }

    #[test]
    fn path_endswith() {
        assert!(super::path_endswith("/A/B/C", "B/C"));
        assert!(!super::path_endswith("/A/B/C", "D/E"));
        assert!(!super::path_endswith("/A/B/C", "A/B"));
    }

    #[test]
    fn svn_resolve_relpath() {
        assert_eq!(super::svn_resolve_relpath("/A/B/C", "../../D"), "/A/D");
    }

    #[test]
    fn subpath_of() {
        assert!(super::subpath_of("A", "A"));
        assert!(super::subpath_of("A/B", "A"));
        assert!(!super::subpath_of("A", "B"));
        assert!(!super::subpath_of("A/B", "A/C"));

        assert!(!super::subpath_of(".", "A"));
        assert!(super::subpath_of("A", "."));
        assert!(super::subpath_of(".", "."));
        assert!(super::subpath_of("./A", "A"));
    }
}
