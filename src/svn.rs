use serde_derive::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/*fn update(
    dev_dir: &str,
    testcase_root_dir: &str,
    testcase_relative_paths: &[String],
    verbose: bool,
) -> bool {
    if !svn_available(verbose) {
        return false;
    }

    let (branch_url, dev_revision) = get_dev_branch_and_revision(&dev_dir, verbose);
    let next_dev_revision = get_next_revision(&branch_url, &dev_revision, verbose);
    return update_revision(
        &branch_url,
        next_dev_revision,
        &testcase_root_dir,
        &testcase_relative_paths,
        verbose,
    );
}*/

/*
fn update_revision(
    branch_url: &str,
    next_dev_revision: Revision,
    testcase_root_dir: &str,
    testcase_relative_paths: &[String],
    verbose: bool,
) -> bool {
    if !svn_available(verbose) {
        return false;
    }

    if verbose {
        println!("Selected branch: {}", branch_url);
        println!("Selected revision: {}", next_dev_revision);
    }

    let testcases_revision = detect_testcases_revision(&branch_url, next_dev_revision);

    let mut wcs = vec![];
    for test_dir in itertools::sorted(testcase_relative_paths) {
        wcs.insert(svn_find_workingcopies(&testcase_root_dir, &test_dir));
    }
    switch_workingcopies(
        &wcs,
        &testcase_root_dir,
        &branch_url,
        testcases_revision,
        verbose,
    );
    return true;
}*/

/*fn checkout(
    dev_dir: &str,
    testcase_root_dir: &str,
    testcase_relative_paths: &[String],
    force_conversion: bool,
    minimal: bool,
    verbose: bool,
) -> bool {
    if !svn_available(verbose) {
        return false;
    }

    let (branch_url, dev_revision) = get_dev_branch_and_revision(&dev_dir, verbose);
    let next_dev_revision = get_next_revision(&branch_url, dev_revision, verbose);
    return checkout_revision(
        &branch_url,
        next_dev_revision,
        &testcase_root_dir,
        &testcase_relative_paths,
        force_conversion,
        minimal,
        verbose,
    );
}*/

/*fn checkout_revision(
    branch_url: &str,
    next_dev_revision: Revision,
    testcase_root_dir: &str,
    testcase_relative_paths: &[String],
    force_conversion: bool,
    minimal: bool,
    verbose: bool,
) -> bool {
    if !svn_available(verbose) {
        return false;
    }

    if verbose {
        println!("Selected branch: {}", branch_url);
        println!("Selected revision: {}", next_dev_revision);
    }

    let testcases_revision = detect_testcases_revision(&branch_url, next_dev_revision);

    let mut depth = svn_depth(&testcase_root_dir);

    match depth {
        None => {
            if !create_checkout_and_convert(
                &testcase_root_dir,
                &branch_url,
                testcases_revision,
                force_conversion,
                verbose,
            ) {
                return false;
            }

            depth = svn_depth(&testcase_root_dir);
            if minimal {
                remove_unneded_testcases(&testcase_root_dir, &testcase_relative_paths, verbose);
            }
        }
        Some(depth) => {
            if minimal {
                // remove before switch to avoid unneded large switches
                remove_unneded_testcases(&testcase_root_dir, &testcase_relative_paths, verbose);
            }
            switch_workingcopies(
                &vec![testcase_root_dir],
                &testcase_root_dir,
                &branch_url,
                testcases_revision,
                verbose,
            );
        }
    }

    if depth == Some("empty") {
        create_missing_testcases(
            &testcase_root_dir,
            &testcase_relative_paths,
            testcases_revision,
            verbose,
        );
    }

    return true;
}*/

/// Check if dev is up to date. If not find the last revision before future dev commits.
/*fn detect_testcases_revision(branch_url: &str, next_dev_revision: Revision) -> Revision {
    let mut testcases_revision = Revision::Head;
    if let Some(Revision::Revision(rev)) = next_dev_revision {
        // Only change HEAD, if we really have testcases commits after the guessed revision.
        // This makes the console output a bit nicer.
        let later_test_logs = SvnLogs(
            branch_url + "/testcases",
            next_dev_revision,
            Revision::Head,
            /*limit=*/ 1,
        )
        .items;
        if !later_test_logs.is_empty() {
            testcases_revision = Revision::Revision(rev - 1);
            println!(
                "Your dev folder is not at the latest revision. The guessed testcases "
                "revision will be wrong, if you committed your testcase changes before "
                "your dev changes.",
            )
        }
    }
    testcases_revision
}*/

/*fn create_checkout_and_convert(
    testcase_root_dir: &str, branch_url: &str, revision: Revision, force_conversion: bool, verbose: bool
) -> bool{
    let nested_checkouts = svn_find_workingcopies(&testcase_root_dir, ".");
    if !nested_checkouts.is_empty() {
        if !force_conversion {
            println!(
                "Aborting because of existing checkouts in testcases. "
                "Use --force to convert them to a single sparse checkout."
            );
            return false
        }

        if verbose {
            println!("Found nested checkouts that need conversion. ")
            println!(
                "Please don't abort or you might have to manually delete your testcases folder!"
            )
        }
        // Switching has to be done before conversion to avoid new local changes
        // when .svn index is deleted
        switch_workingcopies(
            &nested_checkouts, &testcase_root_dir, &branch_url, revision, verbose
        );
    }

    let not_allowed_status = ["conflicted", "unversioned", "added", "deleted", "replaced"]
    for wc in nested_checkouts {
        let status = status(root);
        let has_not_allowed_status = status.target.iter().any(|t| {
            ["conflicted", "unversioned", "added", "deleted", "replaced"]
                .contains(&t.wc_status.item.as_str())
        });
        if has_not_allowed_status {
            println!("Can't proceed because of uncommitted changes in '{}'. "
                "Please solve those manually or delete the whole testcases folder.", wc);
            std::process::exit(-1);
        }
        return false
    }

    if verbose {
        println!("Creating sparse checkout " + testcase_root_dir);
    }
    svn(
        &[
            "checkout",
            "--depth=empty",
            "--force",
            &format!("{}/testcases@{}", branch_url, revision),
            testcase_root_dir,
        ]
    );

    if !nested_checkouts.is_empty() {
        if verbose {
            println!("Converting nested checkouts");
        }
        for wc_path in itertools::sorted(nested_checkouts) {
            let wc_relpath = svn_relpath(wc_path, testcase_root_dir);
            if verbose {
                print_svn_path(wc_relpath);
            }
            delete_svn_index(wc_path);
            // TODO: do nested checkouts always have depth=infinity?
            svn_make_sparse(testcase_root_dir, wc_relpath, revision);
        }
    }
    true
}*/

fn remove_unneded_testcases(
    testcase_root_dir: &str,
    testcase_relative_paths: &[String],
    verbose: bool,
) {
    let mut unneeded_paths = vec![];

    fn recursive_find_unneeded(
        path: &Path,
        testcase_root_dir: &Path,
        testcase_relative_paths: &[PathBuf],
        mut unneeded_paths: &mut Vec<String>,
    ) {
        let abs_path = testcase_root_dir.join(path);
        if !abs_path.exists() || path.to_str().unwrap() == ".svn" || path.ends_with("/.svn") {
            return;
        }
        if !testcase_relative_paths.iter().any(|p| path.ends_with(p)) {
            unneeded_paths.push(path.to_str().unwrap().to_string());
        } else if !testcase_relative_paths.iter().any(|p| p.ends_with(path)) && !abs_path.is_dir() {
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

    let testcase_relative_paths: Vec<_> =
        testcase_relative_paths.iter().map(PathBuf::from).collect();

    recursive_find_unneeded(
        &PathBuf::from("."),
        &PathBuf::from(testcase_root_dir),
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
            let status = status(&path);
            if !status.target.is_empty() {
                println!(
                    "Cannot remove {:?}, it contains changes or unversioned files",
                    path
                );
            } else if !Command::new("svn")
                .args(&["update", "--set-depth=exclude", &path])
                .current_dir(&testcase_root_dir)
                .status()
                .unwrap()
                .success()
            {
                panic!("svn failed!");
            }
        }
    }
}

fn create_missing_testcases(
    testcase_root_dir: &str,
    testcase_relative_paths: &[String],
    revision: Revision,
    verbose: bool,
) {
    let mut missing_paths = vec![];
    for test_path in testcase_relative_paths {
        if svn_depth(test_path, testcase_root_dir).as_deref() != Some("infinity") {
            missing_paths.push(test_path);
        }
    }

    if !missing_paths.is_empty() {
        if verbose {
            println!("Downloading missing testcases");
        }
        for test_path in itertools::sorted(missing_paths) {
            if verbose {
                print_svn_path(&test_path);
            }
            svn_make_sparse(&testcase_root_dir, &test_path, revision);
        }
    }
}

fn switch_workingcopies(
    wcs: &[String],
    testcases_root_path: &str,
    branch_url: &str,
    revision: Revision,
    verbose: bool,
) {
    if verbose {
        println!("Switching {} to {}", testcases_root_path, revision);
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
            let wc_relpath = svn_relpath(&wc, &testcases_root_path);
            if verbose && wc_relpath != "." {
                print_svn_path(&wc_relpath);
            }
            let target_url =
                svn_resolve_relpath(&(branch_url.to_owned() + "/testcases"), &wc_relpath);
            let result = Command::new("svn")
                .args(&[
                    "switch",
                    "--accept=postpone",
                    &format!("{}@{}", target_url, revision),
                    &wc,
                ])
                .output()
                .unwrap();
            if !result.status.success() {
                panic!("svn failed");
            }
            let result = std::str::from_utf8(&result.stdout).unwrap();
            if svn_had_conflicts(result) {
                println!("conflict in {}. Please use svn to resolve it!", wc);
            }
        }
    }
}

fn svn_make_sparse(root: &str, path: &str, revision: Revision) {
    let path_list = path_to_list(path);
    for i in 0..path_list.len() {
        let sub_path = itertools::join(&path_list[0..=i], "/");
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
            if !Command::new("svn")
                .args(&[
                    "update",
                    &format!("--set-depth={}", needed_depth),
                    "--force",
                    "--accept=postpone",
                    &sub_path,
                    "--revision",
                    &revision.to_string(),
                ])
                .current_dir(root)
                .status()
                .expect("SVN failed!")
                .success()
            {
                std::process::exit(1)
            }
            // svn update silently does nothing if an url does not exist.
            // => Check if something was created locally
            if svn_depth(&sub_path, root).is_none() {
                panic!("Path does not exist in SVN. Did you pass the correct test id?");
            }
        }
    }
}

/// Traverses recursively through subdirectories, if no svn working copy found.
/// Validates working copies
fn svn_find_workingcopies(root: &str, relpath: &str) -> Vec<String> {
    let mut relpath_to_wcs = vec![];
    let abs_path = PathBuf::from(root).join(relpath);
    if abs_path.exists() {
        let svn_info = info(&abs_path.to_str().unwrap().replace('\\', "/"));
        if !path_endswith(&svn_info.entry.as_ref().unwrap().url, relpath) {
            println!(
                "Ignoring unexpected subdirectories in svn url {}. Does not fit to {}",
                svn_info.entry.as_ref().unwrap().url,
                relpath
            );
        } else {
            relpath_to_wcs.push(svn_info.entry.as_ref().unwrap().wc_root_path.clone());
        }
        // # Only if the current path is not an svn checkout we search in subdirs.
        // # This misses nested svn checkouts, which is possible but unlikely to happen.
        // if os.path.isdir(abs_path):
        //     for subdir in os.listdir(abs_path):
        //         abs_subdir = os.path.join(abs_path, subdir)
        //         if os.path.isdir(abs_subdir):
        //             relpath_to_wcs += svn_find_workingcopies(
        //                 root, relpath + "/" + subdir
        //             )
    }
    relpath_to_wcs
}

fn delete_svn_index(working_copy_path: &str) {
    let index_path = PathBuf::from(working_copy_path).join(".svn");
    if index_path.is_dir() {
        std::fs::remove_dir_all(index_path).unwrap();
    }
}

fn path_to_list(path: &str) -> Vec<String> {
    // TODO: maybe there is another way to do this with rust os functions
    // TODO: will not work for '..' in the path
    let mut list: Vec<_> = path
        .replace('\\', "/")
        .split('/')
        .filter(|t| *t != ".")
        .map(|t| t.to_string())
        .collect();
    if !list.is_empty() {
        list.pop();
    }
    list
}

fn path_endswith(path: &str, endpath: &str) -> bool {
    let mut split_path = path_to_list(path);
    let mut split_endpath = path_to_list(endpath);
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

fn svn_relpath(local_path: &str, local_base_path: &str) -> String {
    let local_path = PathBuf::from(local_path);
    let relpath = local_path.strip_prefix(local_base_path).unwrap();
    relpath.to_str().unwrap().replace('\\', "/")
}

/// E.g. /A/B/C, ../../D becomes /A/D
fn svn_resolve_relpath(url: &str, relpath: &str) -> String {
    let mut url_list = path_to_list(url);
    let mut relpath_list = path_to_list(relpath);
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

fn svn_depth(local_path: &str, cwd: &str) -> Option<String> {
    info(&local_path).entry.map(|e| e.depth)
}

fn svn_available(verbose: bool) -> bool {
    let output = match Command::new("svn").arg("--version").output() {
        Ok(output) => output,
        Err(e) => {
            if verbose {
                println!(
                    concat!("Could not find svn. Please make sure you installed an svn command line client ",
                    "and put it into the system searh path. For Windows for example install Tortoise SVN ",
                    "and make sure 'command line client tools' are selected.")
                );
            }
            return false;
        }
    };
    let output = std::str::from_utf8(&output.stdout).unwrap();

    let re = regex::Regex::new(r"version (\d+)\.(\d+)").unwrap();
    match re.captures(output) {
        Some(cap) => {
            let (major, minor) = (cap[0].parse().unwrap(), cap[1].parse().unwrap());
            if (major, minor) >= (1, 6) {
                true
            } else {
                if verbose {
                    println!(
                        "Found svn version {}.{}. Please install a version of at least 1.6.",
                        major, minor
                    );
                }
                false
            }
        }
        None => {
            if verbose {
                println!(
                "Could not validate version of svn. Please make sure a recent svn with at least version 1.6 is installed.");
            }
            false
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
    if path.ends_with("/") {
        path = &path[..path.len() - 1];
    }
    println!("  - {}", path);
}

#[derive(Copy, Clone)]
enum Revision {
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

#[derive(Deserialize)]
#[serde(rename = "status")]
struct Status {
    target: Vec<StatusEntry>,
}

#[derive(Deserialize)]
#[serde(rename = "entry")]
struct StatusEntry {
    path: String,
    #[serde(rename = "wc-status")]
    wc_status: WcStatus,
}

#[derive(Deserialize)]
struct WcStatus {
    item: String,
}

fn status(root: &str) -> Status {
    let status = Command::new("svn")
        .args(&["status", "--xml", root])
        .output()
        .expect("SVN failed!");
    if !status.status.success() {
        panic!("SVN failed!");
    }
    let output = std::str::from_utf8(&status.stdout).unwrap();
    serde_xml_rs::from_str(output).unwrap()
}

#[derive(Deserialize)]
#[serde(rename = "info")]
struct Info {
    entry: Option<InfoEntry>,
}

#[derive(Deserialize)]
#[serde(rename = "entry")]
struct InfoEntry {
    url: String,
    depth: String,
    #[serde(rename = "wcroot-abspath")]
    wc_root_path: String,
}

fn info(root: &str) -> Info {
    let status = Command::new("svn")
        .args(&["info", "--xml", root])
        .output()
        .expect("SVN failed!");
    if !status.status.success() {
        panic!("SVN failed!");
    }
    let output = std::str::from_utf8(&status.stdout).unwrap();
    serde_xml_rs::from_str(output).unwrap()
}
