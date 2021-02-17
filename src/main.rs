mod config;
mod report;
mod runnable;
mod scheduler;
mod svn;

use simple_eyre::eyre::{eyre, ContextCompat, Result, WrapErr};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use structopt::StructOpt;

#[global_allocator]
static GLOBAL: std::alloc::System = std::alloc::System;

#[derive(StructOpt)]
#[structopt(name = "mwtest", rename_all = "kebab-case")]
struct Args {
    /// path to a dev folder
    #[structopt(long)]
    dev_dir: Option<String>,

    /// depends on build type, could be a dev folder, a quickstart or a CMake build directory
    #[structopt(long)]
    build_dir: Option<String>,

    /// usually "your-branch/testcases"
    #[structopt(long)]
    testcases_dir: Option<String>,

    #[structopt(long)]
    output_dir: Option<String>,

    /// specifies the type of your build (like dev-windows, quickstart or cmake-windows)
    #[structopt(long)]
    build_type: Option<String>,

    /// specifies which tests to run (like ci, nightly)
    #[structopt(long)]
    preset: Option<String>,

    /// specify build type (ReleaseUnicode, RelWithDebInfo, ...)
    #[structopt(long, short)]
    config: Option<String>,

    #[structopt(subcommand)]
    cmd: SubCommands,
}

#[derive(StructOpt)]
enum SubCommands {
    Build {
        app_names: Vec<String>,

        /// Don't show the XGE monitor for builds.
        #[structopt(long)]
        no_monitor: bool,
    },
    List {
        app_names: Vec<String>,

        /// Show only ids that contain one of the given substrings.
        #[structopt(short, long)]
        filter: Vec<String>,
    },
    Run(RunArgs),
    Info {
        app_name: String,
    },
    Checkout {
        app_names: Vec<String>,

        /// will convert the testcases folder to a sparse checkout
        #[structopt(long)]
        force: bool,

        /// will remove all other checked-out files.
        #[structopt(long)]
        minimal: bool,

        /// the test ids you want to check out
        #[structopt(long)]
        id: Vec<String>,

        /// the test id you want to check out
        //#[structopt(long)]
        //id: Option<String>,

        /// any tests with this substring will be checked out
        //#[structopt(long)]
        //filter: Option<String>,

        /// revision of dev folder (--revision HEAD), overrides revision branch of --dev-dir
        #[structopt(long)]
        revision: Option<String>,

        /// full path to the remote branch (--branch https://svn.moduleworks.com/ModuleWorks/trunk), overrides remote branch of --dev-dir
        #[structopt(long)]
        branch: Option<String>,
    },
    Update {
        app_names: Vec<String>,
    },
}

#[derive(StructOpt)]
pub struct RunArgs {
    app_names: Vec<String>,

    #[structopt(long, conflicts_with = "filter")]
    id: Vec<String>,

    /// Run only ids that contain one of the given substrings.
    #[structopt(short, long)]
    filter: Vec<String>,

    /// Show the full test output, even for succeeded tests.
    #[structopt(short, long)]
    verbose: bool,

    /// Run with multiple local threads. You can also give the thread count explicitly.
    #[structopt(short, long, conflicts_with = "xge")]
    parallel: Option<Option<usize>>,

    #[structopt(long)]
    xge: bool,

    /// Don't open the XGE monitor.
    #[structopt(long)]
    no_monitor: bool,

    /// Run each test 'N' times. All repeats have to succeed.
    #[structopt(short, long, default_value = "1")]
    repeat: usize,

    /// Repeat each test up to 'N' times. At least one of those runs has to succeed
    #[structopt(long, default_value = "1", conflicts_with = "repeat")]
    repeat_if_failed: usize,

    /// MWTest exits with code "0", even if tests failed. This is the expected behavior for Jenkins.
    #[structopt(long)]
    treat_completion_as_success: bool,

    /// abort immediately after the first error
    #[structopt(long)]
    fail_fast: bool,

    /// Disables the timeout check for all tests.
    #[structopt(long)]
    no_timeout: bool,

    #[structopt(long)]
    timeout: Option<f32>,

    /// Multiply all timeouts by this factor.
    #[structopt(long, default_value = "1")]
    timeout_factor: f32,

    /// Test ids named in this file are never run. The format is the same that is printed by "mwtest list".
    /// Lines that begin with '#' are comments.
    #[structopt(long)]
    exclusion_file: Option<String>,

    /// Only changed files are run. They may use a lower timeout
    /// (configured in preset). This is only implemented for tests
    /// where the changed path has a 1-to-1 mapping with the test (like exactoutput).
    #[structopt(long)]
    run_only_changed_file: Option<String>,
}

fn main() -> Result<()> {
    simple_eyre::install()?;

    let mut args = vec![];
    let mut extra_args = vec![]; // TODO: handle extra args
    let mut in_extra_args = false;
    for arg in std::env::args() {
        if arg == "--" {
            in_extra_args = true;
        } else if in_extra_args {
            extra_args.push(arg);
        } else {
            args.push(arg);
        }
    }
    let args = Args::from_iter(args);

    let input_paths = config::InputPaths::from(
        args.dev_dir,
        args.build_dir,
        args.testcases_dir,
        args.build_type,
        args.preset,
        args.config,
    )?;

    let apps_config = config::AppsConfig::load(&input_paths.dev_dir, &input_paths.build_dir)
        .wrap_err("Failed to load apps.json!")?;

    match args.cmd {
        SubCommands::Build {
            app_names,
            no_monitor,
        } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
            cmd_build(&apps, &input_paths, no_monitor)?;
        }
        SubCommands::List { app_names, filter } => {
            if !app_names.is_empty() {
                let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
                let filter_args = FilterArgs {
                    filter: &filter,
                    ids: &[],
                    exclusion_file: &None, // TODO
                };
                let app_tests = generate_app_tests(&filter_args, &input_paths, &apps, false);
                cmd_list_tests(&app_tests);
            } else {
                cmd_list_apps(&apps_config);
            }
        }
        SubCommands::Run(run_args) => {
            let apps = apps_config.select_build_and_preset(&run_args.app_names, &input_paths)?;
            let can_run_raw_gtest = run_args.filter.is_empty()
                && run_args.id.is_empty()
                && run_args.parallel.is_none()
                && !run_args.xge
                && run_args.repeat_if_failed == 0;
            let filter_args = FilterArgs {
                filter: &run_args.filter,
                ids: &run_args.id,
                exclusion_file: &run_args.exclusion_file,
            };
            let app_tests =
                generate_app_tests(&filter_args, &input_paths, &apps, can_run_raw_gtest);
            let out_dir = args
                .output_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap().join("test_output"));
            let output_paths = OutputPaths {
                out_dir: out_dir.clone(),
                tmp_dir: out_dir.join("tmp"),
            };
            let success = cmd_run(&input_paths, &app_tests, &output_paths, &run_args)?
                || run_args.treat_completion_as_success;
            if !success {
                std::process::exit(-1)
            }
        }
        SubCommands::Info { app_name } => {
            cmd_info(app_name, &apps_config, &input_paths)?;
        }
        SubCommands::Checkout {
            app_names,
            force,
            minimal,
            id,
            //filter,
            revision,
            branch,
        } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;

            let mut paths: Vec<String> = vec![];
            for app in apps.0.values() {
                for test in &app.tests {
                    for group in &test.groups {
                        if let Some(expr) = &group.find_glob {
                            if id.is_empty() {
                                // check out all test files
                                let path = expr.split('*').next().unwrap();
                                paths.push(path.to_string());
                            } else {
                                // reconstruct paths by filling the id into the id_pattern
                                for id in &id {
                                    let mut path = test
                                        .id_pattern
                                        .as_ref()
                                        .map(|p| p.replace("(.*)", &id))
                                        .unwrap_or_else(|| id.to_string());
                                    if app.checkout_parent {
                                        path = relative_path::RelativePath::new(&path)
                                            .parent()
                                            .unwrap()
                                            .to_string();
                                    }
                                    paths.push(path);
                                }
                            }
                        }
                        for path in &group.testcases_dependencies {
                            paths.push(path.to_string());
                        }
                        if let Some(exclusion_list) = &group.exclusion_list {
                            paths.push(exclusion_list.to_string());
                        }
                    }
                }
            }

            if input_paths.dev_dir.is_none() && branch.is_none() {
                println!("ERROR: you need to specify the dev directory (--dev-dir) or a remote branch (--branch)");
                println!("Note: the --dev-dir parameter needs to appear before the subcommand \"checkout\"");
                std::process::exit(-1)
            }
            let dev_dir = input_paths.dev_dir.as_ref().unwrap();
            match (branch, revision) {
                (Some(branch), Some(revision)) => {
                    let revision = match revision.as_ref() {
                        "HEAD" => svn::Revision::Head,
                        r => svn::Revision::Revision(
                            r.parse().wrap_err("Failed to parse given revision.")?,
                        ),
                    };
                    svn::checkout_revision(
                        &branch,
                        revision,
                        &input_paths.testcases_dir,
                        &paths,
                        force,
                        minimal,
                        true,
                    )?;
                }
                _ => {
                    svn::checkout(
                        &dev_dir,
                        &input_paths.testcases_dir,
                        &paths,
                        force,
                        minimal,
                        true,
                    )?;
                }
            }
        }
        SubCommands::Update { app_names } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
            let mut paths: Vec<String> = vec![];
            for app in apps.0.values() {
                for test in &app.tests {
                    for group in &test.groups {
                        if let Some(expr) = &group.find_glob {
                            paths.push(expr.split('*').next().unwrap().to_string());
                        }
                        for path in &group.testcases_dependencies {
                            paths.push(path.to_string());
                        }
                    }
                }
            }

            if input_paths.dev_dir.is_none() {
                println!("ERROR: you need to specify the dev directory (--dev-dir).");
                println!("Note: the --dev-dir parameter needs to appear before the subcommand \"update\"");
                std::process::exit(-1)
            }
            svn::update(
                &input_paths.dev_dir.as_ref().unwrap(),
                &input_paths.testcases_dir,
                &paths,
                true,
            )?;
        }
    }

    Ok(())
}

fn cmd_build(apps: &config::Apps, paths: &config::InputPaths, no_monitor: bool) -> Result<()> {
    let mut dependencies: HashMap<String, Vec<&str>> = HashMap::new();
    for (name, app) in apps.0.iter() {
        let build = &app.build;
        let solution = build.solution.clone().unwrap_or_else(|| {
            paths
                .build_dir
                .join("mwBuildAll.sln")
                .to_str()
                .unwrap()
                .to_owned()
        });
        let project = build
            .project
            .as_ref()
            .wrap_err_with(|| format!("no project defined for {}", name))?;
        let deps = dependencies.entry(solution).or_insert_with(Vec::new);
        (*deps).push(project);
    }
    let has_buildconsole = match Command::new("buildConsole").output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        _ => true,
    };
    for (solution, projects) in dependencies {
        let projects = projects.join(",");
        let status = if has_buildconsole {
            println!(
                "building:\n  solution: {}\n  projects: {}",
                &solution, &projects
            );
            let mut command = Command::new("buildConsole");
            command
                .arg(solution)
                .arg("/build")
                .arg("/silent")
                .arg(format!("/cfg={}|x64", paths.build_config))
                .arg(format!("/prj={}", projects));
            if !no_monitor {
                command.arg("/openmonitor");
            }
            command.status().wrap_err("failed to build project!")?
        } else {
            println!("building:\n  projects: {}", &projects);
            Command::new("cmake")
                .arg("--build")
                .arg(&paths.build_dir)
                .arg("--config")
                .arg(&paths.build_config)
                .arg("--target")
                .arg(&projects)
                .status()
                .wrap_err("failed to build project!")?
        };
        if status.success() {
            println!("Build succeeded.");
        } else {
            println!("Build failed.");
        }
    }
    Ok(())
}

fn cmd_list_apps(apps: &config::AppsConfig) {
    for name in apps.app_names() {
        println!("  {}", name);
    }
}

fn cmd_list_tests(apps: &[AppWithTests]) {
    for app in apps {
        for group in &app.tests {
            for test_id in &group.test_ids {
                println!("{} --id {}", app.name, test_id.id);
            }
        }
    }
}

fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &[AppWithTests],
    output_paths: &OutputPaths,
    run_args: &crate::RunArgs,
) -> Result<bool> {
    if Path::exists(&output_paths.out_dir) {
        if !Path::exists(&output_paths.out_dir.clone().join("results.xml")) {
            return Err(eyre!(
                "ERROR: can't reset the output directory: {:?}.\n It doesn't look like it \
                 was created by mwtest. Please select another one or delete it manually.",
                &output_paths.out_dir
            ));
        }
        remove_dir_all::remove_dir_all(&output_paths.out_dir)
            .expect("could not clean up tmp directory!");
    }
    std::fs::create_dir_all(&output_paths.tmp_dir).expect("could not create tmp directory!");
    println!(
        "Test artifacts will be written to {}.",
        output_paths.out_dir.to_str().unwrap()
    );

    let tests = runnable::create_run_commands(&input_paths, &test_apps, &output_paths, &run_args);
    if tests.is_empty() {
        println!("WARNING: No tests were selected.");
        std::process::exit(0); // counts as success
    }
    scheduler::run(&input_paths, tests, &output_paths, &run_args)
}

fn cmd_info(
    name: String,
    apps_config: &config::AppsConfig,
    input_paths: &config::InputPaths,
) -> Result<()> {
    let apps = apps_config
        .clone()
        .select_build_and_preset(&[name], &input_paths)?;
    for (name, app) in &apps.0 {
        let cfg = &apps_config.0[name];

        println!("App: {}", name);
        println!("Aliases: {:?}", cfg.alias);
        println!("Tags: {:?}", cfg.tags);
        println!("Responsible: {}", app.responsible);
        if cfg.disabled {
            println!(
                r#"
===============================
This test is currently disabled
==============================="#
            )
        }
        println!("Build:");
        println!(
            "  Config: {} ({:?})",
            input_paths.build_config, input_paths.build_type
        );
        println!("  Executable: {}", app.build.exe);
        println!(
            "  Working directory: {}",
            app.build.cwd.as_deref().unwrap_or("<default>")
        );
        println!("  Solution: {:?}", app.build.solution);
        println!("  Project: {:?}", app.build.project);

        println!("Test Files:");

        for test_preset in &app.tests {
            for group in &test_preset.groups {
                if let Some(g) = &group.find_glob {
                    println!("    files: {}", g);
                }
                if let Some(g) = &group.find_gtest {
                    println!("    gtests: {}", g);
                }
                println!("    execution style: {}", group.execution_style);
                println!("    timeout: {:?}", group.timeout);
                println!("    timeout if changed: {:?}", group.timeout_if_changed);
                // generates_outputs
                println!(
                    "    testcases dependencies: {:?}",
                    group.testcases_dependencies
                );
                println!("    command: {:?}", group.command);
            }
        }

        /*for preset_name in input_paths.preset.split('+') {
            if let Some(preset) = cfg.tests.get(preset_name) {
                println!("  Preset: {}", preset_name);
                for group in preset.groups {

                }
            }
        }*/
    }
    Ok(())
}

#[derive(Debug)]
pub struct AppWithTests {
    name: String,
    app: config::App,
    tests: Vec<GroupWithTests>,
}

#[derive(Debug)]
struct GroupWithTests {
    test_group: config::TestGroup,
    command: config::CommandTemplate,
    test_ids: Vec<TestId>,
    test_filter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TestId {
    pub id: String,
    pub rel_path: Option<PathBuf>,
}
impl std::hash::Hash for TestId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

#[derive(Debug, Clone)]
pub struct OutputPaths {
    out_dir: PathBuf,
    tmp_dir: PathBuf,
}

struct FilterArgs<'a> {
    filter: &'a [String],
    ids: &'a [String],
    exclusion_file: &'a Option<String>,
}
fn generate_app_tests(
    filter_args: &FilterArgs,
    input_paths: &config::InputPaths,
    apps_config: &config::Apps,
    can_run_raw_gtest: bool,
) -> Vec<AppWithTests> {
    let id_filter = id_filter_from_args(&filter_args).unwrap();
    let apps: Vec<AppWithTests> = apps_config
        .0
        .iter()
        .map(|(app_name, app)| {
            // populate with tests
            let tests: Vec<GroupWithTests> = app
                .tests
                .iter()
                .flat_map(|preset_config| {
                    preset_config
                        .groups
                        .iter()
                        .map(|test_group| {
                            let test_filter = if can_run_raw_gtest && app.supports_gtest_batching {
                                test_group.find_gtest.clone()
                            } else {
                                None
                            };
                            GroupWithTests {
                                test_group: test_group.clone(),
                                command: test_group.command.clone(),
                                test_ids: test_group
                                    .generate_test_inputs(&app, &preset_config, &input_paths)
                                    .into_iter()
                                    .filter(|f| id_filter(&f.id))
                                    .collect(),
                                test_filter,
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
            AppWithTests {
                name: app_name.to_string(),
                app: (*app).clone(),
                tests,
            }
        })
        .filter(|app_with_tests| !app_with_tests.tests.is_empty())
        .collect();
    if apps.is_empty() {
        println!("WARNING: you have not selected any tests.");
    }
    apps
}

fn id_filter_from_args<'a>(
    filter_args: &'a FilterArgs<'a>,
) -> Result<Box<dyn Fn(&str) -> bool + 'a>> {
    let normalize = |input: &str| input.to_lowercase().replace('\\', "/");
    let exclusion_filter =
        id_filter_from_exclusion_file(&filter_args).wrap_err("while loading exclusion file")?;
    Ok(Box::new(move |input: &str| {
        let input = normalize(input);
        if !exclusion_filter(&input) {
            return false;
        }
        if !filter_args.filter.is_empty() {
            filter_args
                .filter
                .iter()
                .any(|f| input.contains(normalize(f).as_str()))
        } else if !filter_args.ids.is_empty() {
            filter_args.ids.iter().any(|i| normalize(i) == input)
        } else {
            true
        }
    }))
}

fn id_filter_from_exclusion_file(run_args: &FilterArgs) -> Result<Box<dyn Fn(&str) -> bool>> {
    if let Some(exclusion_file) = run_args.exclusion_file {
        let mut excluded = vec![];
        let content = std::fs::read_to_string(exclusion_file)?;
        for line in content.lines() {
            let mut tokens = line.split(" --id ");
            match (tokens.next(), tokens.next()) {
                (Some(app), Some(id)) => {
                    excluded.push((
                        app.trim().to_string(),
                        id.trim().trim_matches('"').to_string(),
                    ));
                }
                _ => {} // ignore
            }
        }
        Ok(Box::new(move |input: &str| {
            !excluded.iter().any(|(_app, id)| id == input)
        }))
    } else {
        Ok(Box::new(move |_input: &str| true))
    }
}
