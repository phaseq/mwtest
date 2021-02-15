mod config;
mod report;
mod runnable;
mod scheduler;
mod svn;

use color_eyre::eyre::{eyre, ContextCompat, Result, WrapErr};
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
    },
    List {
        app_names: Vec<String>,
        /// select ids that contain one of the given substrings
        #[structopt(short, long)]
        filter: Vec<String>,
    },
    Run {
        app_names: Vec<String>,

        #[structopt(long, conflicts_with = "filter")]
        id: Vec<String>,

        /// select ids that contain one of the given substrings
        #[structopt(short, long)]
        filter: Vec<String>,

        #[structopt(short, long)]
        verbose: bool,

        #[structopt(short, long, conflicts_with = "xge")]
        parallel: bool,
        #[structopt(long)]
        xge: bool,

        #[structopt(short, long, default_value = "1")]
        repeat: usize,
        #[structopt(long, default_value = "0")]
        repeat_if_failed: usize,

        #[structopt(long)]
        no_timeout: bool,
    },
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

        /// the test id you want to check out
        //#[structopt(long)]
        //id: Option<String>,

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

fn main() -> Result<()> {
    color_eyre::install()?;

    let args = Args::from_args();

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
        SubCommands::Build { app_names } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
            cmd_build(&apps, &input_paths)?;
        }
        SubCommands::List { app_names, filter } => {
            if !app_names.is_empty() {
                let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
                let app_tests = generate_app_tests(filter, vec![], &input_paths, &apps, false);
                cmd_list_tests(&app_tests);
            } else {
                cmd_list_apps(&apps_config);
            }
        }
        SubCommands::Run {
            app_names,
            id,
            filter,
            verbose,
            parallel,
            xge,
            repeat,
            repeat_if_failed,
            no_timeout,
        } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
            let can_run_raw_gtest =
                filter.is_empty() && id.is_empty() && !parallel && !xge && repeat_if_failed == 0;
            let app_tests = generate_app_tests(filter, id, &input_paths, &apps, can_run_raw_gtest);
            let out_dir = args
                .output_dir
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap().join("test_output"));
            let output_paths = OutputPaths {
                out_dir: out_dir.clone(),
                tmp_dir: out_dir.join("tmp"),
            };
            let repeat_strategy = if repeat_if_failed != 0 {
                scheduler::RepeatStrategy::RepeatIfFailed(repeat_if_failed)
            } else {
                scheduler::RepeatStrategy::Repeat(repeat)
            };
            let run_config = scheduler::RunConfig {
                verbose,
                parallel,
                xge,
                repeat: repeat_strategy,
            };
            let success = cmd_run(
                &input_paths,
                &app_tests,
                &output_paths,
                &run_config,
                no_timeout,
            )?;
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
            //id,
            //filter,
            revision,
            branch,
        } => {
            let apps = apps_config.select_build_and_preset(&app_names, &input_paths)?;
            //let id_filter = id_filter_from_args(filter, id);

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

fn cmd_build(apps: &config::Apps, paths: &config::InputPaths) -> Result<()> {
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
            Command::new("buildConsole")
                .arg(solution)
                .arg("/build")
                .arg("/silent")
                .arg(format!("/cfg={}|x64", paths.build_config))
                .arg(format!("/prj={}", projects))
                .arg("/openmonitor")
                .status()
                .wrap_err("failed to build project!")?
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
    run_config: &scheduler::RunConfig,
    no_timeout: bool,
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

    let tests = runnable::create_run_commands(&input_paths, &test_apps, &output_paths, no_timeout);
    if tests.is_empty() {
        println!("WARNING: No tests were selected.");
        std::process::exit(0); // counts as success
    }
    scheduler::run(&input_paths, tests, &output_paths, &run_config)
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

fn generate_app_tests(
    filter: Vec<String>,
    id: Vec<String>,
    input_paths: &config::InputPaths,
    apps_config: &config::Apps,
    can_run_raw_gtest: bool,
) -> Vec<AppWithTests> {
    let id_filter = id_filter_from_args(filter, id);
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
                            let test_filter = if can_run_raw_gtest {
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

fn id_filter_from_args(filter: Vec<String>, id: Vec<String>) -> Box<dyn Fn(&str) -> bool> {
    let normalize = |input: &str| input.to_lowercase().replace('\\', "/");
    Box::new(move |input: &str| {
        let input = normalize(input);
        if !filter.is_empty() {
            filter.iter().any(|f| input.contains(normalize(f).as_str()))
        } else if !id.is_empty() {
            id.iter().any(|i| normalize(i) == input)
        } else {
            true
        }
    })
}
