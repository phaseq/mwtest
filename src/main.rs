mod config;
mod report;
use crate::config::CommandTemplate;
use clap::{App, AppSettings, Arg, ArgGroup, SubCommand};
use scoped_threadpool::Pool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use uuid::Uuid;

#[global_allocator]
static GLOBAL: std::alloc::System = std::alloc::System;

fn main() {
    let registered_tests = config::InputPaths::get_registered_tests();
    let registered_tests_str: Vec<&str> = registered_tests.iter().map(|s| s.as_str()).collect();
    let test_app_arg = Arg::with_name("test_app")
        .required(true)
        .multiple(true)
        .possible_values(&registered_tests_str[..]);
    let filter_arg = Arg::with_name("filter")
        .short("f")
        .long("filter")
        .takes_value(true)
        .help("select ids that contain one of the given substrings")
        .multiple(true);
    let matches = App::new("MW Test")
        .setting(AppSettings::SubcommandRequired)
        .arg(Arg::with_name("BUILD_ROOT")
            .long("build-dir")
            .takes_value(true)
            .help("depends on build type, could be \"your-branch/dev\", a quickstart or a CMake build directory"))
        .arg(Arg::with_name("TESTCASES_ROOT")
            .long("testcases-dir")
            .takes_value(true)
            .help("usually \"your-branch/testcases\""))
        .arg(Arg::with_name("OUT_DIR")
            .long("output-dir")
            .takes_value(true))
        .arg(Arg::with_name("BUILD_LAYOUT")
            .short("b")
            .long("--build")
            .takes_value(true)
            .help("specifies the layout of your build (like dev-releaseunicode, quickstart or cmake)"))
        .arg(Arg::with_name("PRESET")
            .long("--preset")
            .takes_value(true)
            .help("specifies which tests to run (like ci, nightly)"))
        .subcommand(
            SubCommand::with_name("build")
                .arg(test_app_arg.clone()),
        ).subcommand(
            SubCommand::with_name("list")
                .arg(test_app_arg.clone())
                .arg(filter_arg.clone()),
        ).subcommand(
            SubCommand::with_name("run")
                .arg(test_app_arg)
                .arg(filter_arg)
                .arg(Arg::with_name("id")
                    .long("id")
                    .takes_value(true))
                .group(ArgGroup::with_name("filter_group")
                    .args(&["filter", "id"]))
                .arg(Arg::with_name("verbose")
                    .short("v")
                    .long("verbose"))
                .arg(Arg::with_name("threadpool")
                    .short("p")
                    .long("parallel")
                    .help("run using local thread pool"))
                .arg(Arg::with_name("xge")
                    .long("xge"))
                .group(ArgGroup::with_name("parallel")
                    .args(&["threadpool", "xge"]))
                .arg(Arg::with_name("repeat")
                    .long("repeat")
                    .takes_value(true)
                    .default_value("1"))
                .arg(Arg::with_name("RERUN_IF_FAILED")
                    .long("repeat-if-failed")
                    .takes_value(true)
                    .default_value("0"))
        ).get_matches();

    let input_paths = config::InputPaths::from(
        &matches.value_of("BUILD_ROOT"),
        &matches.value_of("TESTCASES_ROOT"),
        &matches.value_of("BUILD_LAYOUT"),
        &matches.value_of("PRESET"),
    );

    if let Some(matches) = matches.subcommand_matches("build") {
        cmd_build(
            &matches
                .values_of("test_app")
                .unwrap()
                .collect::<Vec<&str>>(),
            &input_paths,
        );
        std::process::exit(0);
    }

    let test_group_file = match config::TestGroupFile::open(&input_paths.preset_path) {
        Ok(file) => file,
        Err(e) => {
            println!(
                "ERROR: failed to parse preset file at {:?}: {}",
                &input_paths.preset_path, e
            );
            std::process::exit(-1);
        }
    };

    if let Some(matches) = matches.subcommand_matches("list") {
        let test_apps = test_apps_from_args(&matches, &input_paths, &test_group_file);
        cmd_list(&test_apps);
    } else if let Some(matches) = matches.subcommand_matches("run") {
        let test_apps = test_apps_from_args(&matches, &input_paths, &test_group_file);
        let out_dir = matches
            .value_of("OUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap().join("test_output"));
        let output_paths = OutputPaths {
            out_dir: out_dir.clone(),
            tmp_dir: out_dir.join("tmp"),
        };
        let run_config = RunConfig {
            verbose: matches.is_present("verbose"),
            parallel: matches.is_present("threadpool"),
            xge: matches.is_present("xge"),
            repeat: matches
                .value_of("repeat")
                .unwrap()
                .parse()
                .expect("expected numeric value for repeat"),
            rerun_if_failed: matches
                .value_of("RERUN_IF_FAILED")
                .unwrap()
                .parse()
                .expect("expected numeric value for repeat-if-failed"),
        };
        let success = cmd_run(&input_paths, &test_apps, &output_paths, &run_config);
        if !success {
            std::process::exit(-1)
        }
    }
}

fn cmd_build(test_names: &[&str], input_paths: &config::InputPaths) {
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, dep) in test_names
        .iter()
        .map(|n| (n, &input_paths.apps.get(*n).unwrap().layout))
    {
        if dep.solution.is_none() || dep.project.is_none() {
            println!("ERROR: no solution/project defined for {}", name);
            std::process::exit(-1);
        }
        let deps = dependencies
            .entry(dep.solution.as_ref().unwrap())
            .or_insert_with(Vec::new);
        (*deps).push(dep.project.as_ref().unwrap());
    }
    for (solution, projects) in dependencies {
        let projects = projects.join(",");
        println!("building:\n  solution: {}\n  {}", &solution, &projects);
        Command::new("buildConsole")
            .arg(solution)
            .arg("/build")
            .arg("/silent")
            .arg("/cfg=ReleaseUnicode|x64")
            .arg(format!("/prj={}", projects))
            .arg("/openmonitor")
            .spawn()
            .expect("failed to launch buildConsole!")
            .wait()
            .expect("failed to build project!");
    }
}

fn cmd_list(test_apps: &[AppWithTests]) {
    for test_app in test_apps {
        for group in &test_app.tests {
            for test_id in &group.test_ids {
                println!("{} --id {}", test_app.name, test_id.id);
            }
        }
    }
}

fn cmd_run(
    input_paths: &config::InputPaths,
    test_apps: &[AppWithTests],
    output_paths: &OutputPaths,
    run_config: &RunConfig,
) -> bool {
    if Path::exists(&output_paths.out_dir) {
        if !Path::exists(&output_paths.out_dir.clone().join("results.xml")) {
            println!(
                "ERROR: can't reset the output directory: {:?}.\n It doesn't look like it \
                 was created by mwtest. Please select another one or delete it manually.",
                &output_paths.out_dir
            );
            std::process::exit(-1);
        }
        std::fs::remove_dir_all(&output_paths.out_dir).expect("could not clean up tmp directory!");
    }
    std::fs::create_dir_all(&output_paths.tmp_dir).expect("could not create tmp directory!");
    println!(
        "Test artifacts will be written to {}.",
        output_paths.out_dir.to_str().unwrap()
    );

    let tests = create_run_commands(&input_paths, &test_apps, &output_paths);

    let n_workers = if run_config.parallel {
        num_cpus::get()
    } else if run_config.xge {
        2 + num_cpus::get()
    } else {
        1
    };
    let mut pool = Pool::new(n_workers as u32);
    pool.scoped(|scope| run_in_scope(scope, &tests, &input_paths, &output_paths, &run_config))
}

fn run_in_scope<'scope>(
    scope: &scoped_threadpool::Scope<'_, 'scope>,
    tests: &'scope [TestInstanceCreator<'_>],
    input_paths: &'scope config::InputPaths,
    output_paths: &'scope OutputPaths,
    run_config: &'scope RunConfig,
) -> bool {
    let mut report = report::Report::new(
        &output_paths.out_dir,
        input_paths.testcases_root.to_str().unwrap(),
        run_config.verbose,
    );

    let mut n = tests.len() * run_config.repeat;

    let (tx, rx) = mpsc::channel();
    let (xge_tx, xge_rx) = mpsc::channel::<TestInstance<'_>>();
    if run_config.xge {
        launch_xge_management_threads(scope, xge_rx, &tx, &output_paths);
    }

    let mut run_counts = HashMap::new();
    for test_instance_generator in tests.iter() {
        run_counts.insert(test_instance_generator.get_uid(), RunCount::new());
        for _ in 0..run_config.repeat {
            run_test_instance(
                test_instance_generator.instantiate(),
                scope,
                &xge_tx,
                &tx,
                &output_paths,
                &run_config,
            );
        }
    }

    let mut i = 0;
    while i < n {
        let (test_instance, output) = match rx.recv_timeout(std::time::Duration::from_secs(6 * 60))
        {
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => panic!("test executor failed!"),
            Ok(result) => result,
        };
        let test_uid = test_instance.get_uid();
        let run_count = run_counts.get_mut(&test_uid).unwrap();
        run_count.n_runs += 1;
        i += 1;
        if output.exit_code == 0 {
            run_count.n_successes += 1;
        } else if run_count.n_runs <= run_config.rerun_if_failed {
            n += 1;
            let test_instance_generator = tests.iter().find(|t| t.get_uid() == test_uid).unwrap();

            run_test_instance(
                test_instance_generator.instantiate(),
                scope,
                &xge_tx,
                &tx,
                &output_paths,
                &run_config,
            );
        }
        report.add(i, n, test_instance, &output);
    }

    drop(report);

    report_and_check_runs(&run_counts)
}

fn run_test_instance<'scope>(
    test_instance: TestInstance<'scope>,
    scope: &scoped_threadpool::Scope<'_, 'scope>,
    xge_tx: &mpsc::Sender<TestInstance<'scope>>,
    tx: &mpsc::Sender<(TestInstance<'scope>, TestCommandResult)>,
    output_paths: &'scope OutputPaths,
    run_config: &'scope RunConfig,
) {
    if run_config.xge && test_instance.allow_xge {
        xge_tx
            .send(test_instance)
            .expect("channel did not accept test input!");
    } else {
        let tx = tx.clone();
        scope.execute(move || {
            let output = test_instance.run(&output_paths);
            tx.send((test_instance, output))
                .expect("channel did not accept test result!");
        });
    }
}

struct RunCount {
    n_runs: usize,
    n_successes: usize,
}
impl RunCount {
    fn new() -> RunCount {
        RunCount {
            n_runs: 0,
            n_successes: 0,
        }
    }
}
fn report_and_check_runs(run_counts: &HashMap<TestUid<'_>, RunCount>) -> bool {
    let test_formatter = |(id, run_counts): (&TestUid<'_>, &RunCount)| {
        if run_counts.n_runs > 1 {
            format!(
                "  {} --id {} (succeeded {} out of {} runs)",
                id.0, id.1, run_counts.n_successes, run_counts.n_runs
            )
        } else {
            format!("  {} --id {}", id.0, id.1)
        }
    };
    let mut failed: Vec<String> = run_counts
        .iter()
        .filter(|(_id, run_counts)| run_counts.n_successes == 0)
        .map(test_formatter)
        .collect();
    failed.sort_unstable();
    let all_succeeded = failed.is_empty();

    let mut instable: Vec<String> = run_counts
        .iter()
        .filter(|(_id, run_counts)| {
            run_counts.n_successes > 0 && run_counts.n_successes < run_counts.n_runs
        })
        .map(test_formatter)
        .collect();
    instable.sort_unstable();
    let none_instable = instable.is_empty();

    if !none_instable {
        println!("Tests that are instable: ");
        for t in instable {
            println!("{}", t);
        }
    }

    if !all_succeeded {
        println!("Tests that failed: ");
        for t in failed {
            println!("{}", t);
        }
    }

    if all_succeeded && none_instable {
        println!("All tests succeeded!");
    }

    all_succeeded
}

type ResultMessage<'a> = (TestInstance<'a>, TestCommandResult);
fn launch_xge_management_threads<'pool, 'scope>(
    scope: &scoped_threadpool::Scope<'pool, 'scope>,
    xge_rx: mpsc::Receiver<TestInstance<'scope>>,
    tx: &mpsc::Sender<ResultMessage<'scope>>,
    output_paths: &'scope OutputPaths,
) {
    let tx = tx.clone();
    let (mut xge_writer, xge_reader) = xge_lib::xge();
    let issued_commands: Arc<Mutex<Vec<TestInstance<'_>>>> = Arc::new(Mutex::new(Vec::new()));
    let issued_commands2 = issued_commands.clone();
    scope.execute(move || {
        for test_instance in xge_rx.iter() {
            let request = {
                let mut locked_issued_commands = issued_commands.lock().unwrap();
                let request = xge_lib::StreamRequest {
                    id: locked_issued_commands.len() as u64,
                    title: test_instance.test_id.id.clone(),
                    cwd: test_instance.command.cwd.clone(),
                    command: test_instance.command.command.clone(),
                    local: false,
                };
                locked_issued_commands.push(test_instance);
                request
            };

            xge_writer
                .run(&request)
                .expect("error in xge.run(): could not send command");
        }
        xge_writer
            .done()
            .expect("error in xge.done(): could not close socket");
    });
    scope.execute(move || {
        for stream_result in xge_reader {
            let result = TestCommandResult {
                exit_code: stream_result.exit_code,
                stdout: stream_result.stdout,
            };
            let success = result.exit_code == 0;
            let locked_issued_commands = issued_commands2.lock().unwrap();
            let test_instance = &locked_issued_commands[stream_result.id as usize];
            let message = (test_instance.clone(), result);
            tx.send(message)
                .expect("error in mpsc: could not send result");
            test_instance
                .cleanup(success, &output_paths)
                .expect("failed to clean up temporary output directory!");
        }
    });
}

struct RunConfig {
    verbose: bool,
    parallel: bool,
    xge: bool,
    repeat: usize,
    rerun_if_failed: usize,
}

#[derive(Debug)]
struct AppWithTests {
    name: String,
    app: config::App,
    tests: Vec<GroupWithTests>,
}

#[derive(Debug)]
struct GroupWithTests {
    test_group: config::TestGroup,
    test_ids: Vec<TestId>,
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

#[derive(Debug)]
struct OutputPaths {
    out_dir: PathBuf,
    tmp_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct TestCommand {
    command: Vec<String>,
    cwd: String,
    pub tmp_path: Option<PathBuf>,
}
type CommandGenerator = dyn Fn() -> TestCommand;

#[derive(Debug, Clone)]
pub struct TestCommandResult {
    exit_code: i32,
    stdout: String,
}

fn create_run_commands<'a>(
    input_paths: &config::InputPaths,
    test_apps: &'a [AppWithTests],
    output_paths: &OutputPaths,
) -> Vec<TestInstanceCreator<'a>> {
    let mut tests: Vec<TestInstanceCreator<'a>> = Vec::new();
    for app in test_apps {
        for group in &app.tests {
            for test_id in &group.test_ids {
                let (input_str, cwd) = test_id_to_input(&test_id, &input_paths, &app.app);
                let generator = test_command_generator(
                    &app.app.properties.command_template,
                    &input_str,
                    cwd,
                    output_paths.tmp_dir.clone(),
                );
                tests.push(TestInstanceCreator {
                    app_name: &app.name,
                    test_id: &test_id,
                    allow_xge: group.test_group.xge,
                    command_generator: generator,
                });
            }
        }
    }
    tests
}

fn test_id_to_input(
    test_id: &TestId,
    input_paths: &config::InputPaths,
    app: &config::App,
) -> (String, String) {
    if let Some(rel_path) = &test_id.rel_path {
        let full_path = input_paths.testcases_root.join(&rel_path);
        if full_path.is_dir() {
            // machsim case
            let full_path = input_paths.testcases_root.join(&rel_path);
            (
                full_path.to_str().unwrap().to_string(),
                full_path.to_str().unwrap().to_string(),
            )
        } else if let Some(cwd) = &app.layout.cwd {
            // cncsim case
            (full_path.to_str().unwrap().to_string(), cwd.clone())
        } else {
            // verifier case
            let file_name = rel_path.file_name().unwrap().to_str().unwrap().to_string();
            let parent_dir = full_path.parent().unwrap().to_str().unwrap().to_string();
            (file_name, parent_dir)
        }
    } else {
        // gtest case
        (
            test_id.id.clone(),
            app.layout
                .cwd
                .clone()
                .expect("You need to specify a CWD for gtests (see preset)."),
        )
    }
}

fn test_command_generator(
    command_template: &CommandTemplate,
    input: &str,
    cwd: String,
    tmp_root: PathBuf,
) -> Box<CommandGenerator> {
    let command = command_template.apply("{{input}}", &input);
    if command.has_pattern("{{tmp_path}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_str().unwrap().to_string();
            let command = command.apply("{{tmp_path}}", &tmp_path);
            std::fs::create_dir(&tmp_path).expect("could not create tmp path!");
            TestCommand {
                command: command.0.clone(),
                cwd: cwd.to_string(),
                tmp_path: Some(tmp_dir),
            }
        })
    } else if command.has_pattern("{{tmp_file}}") {
        Box::new(move || {
            let tmp_dir = tmp_root.join(PathBuf::from(Uuid::new_v4().to_string()));
            let tmp_path = tmp_dir.to_str().unwrap().to_string();
            let command = command.apply("{{tmp_file}}", &tmp_path);
            TestCommand {
                command: command.0.clone(),
                cwd: cwd.to_string(),
                tmp_path: Some(tmp_dir),
            }
        })
    } else {
        Box::new(move || TestCommand {
            command: command.0.clone(),
            cwd: cwd.to_string(),
            tmp_path: None,
        })
    }
}

type TestUid<'a> = (&'a str, &'a str);

struct TestInstanceCreator<'a> {
    app_name: &'a str,
    test_id: &'a TestId,
    allow_xge: bool,
    command_generator: Box<CommandGenerator>,
}
impl<'a> TestInstanceCreator<'a> {
    fn instantiate(&self) -> TestInstance<'a> {
        TestInstance {
            app_name: self.app_name,
            test_id: self.test_id,
            allow_xge: self.allow_xge,
            command: (self.command_generator)(),
        }
    }
    fn get_uid(&self) -> TestUid<'a> {
        (self.app_name, &self.test_id.id)
    }
}
#[derive(Debug, Clone)]
pub struct TestInstance<'a> {
    pub app_name: &'a str,
    test_id: &'a TestId,
    allow_xge: bool,
    pub command: TestCommand,
}
impl<'a> TestInstance<'a> {
    fn run(&self, output_paths: &OutputPaths) -> TestCommandResult {
        let maybe_output = Command::new(&self.command.command[0])
            .args(self.command.command[1..].iter())
            .current_dir(&self.command.cwd)
            .output();
        let output = match maybe_output {
            Ok(output) => output,
            Err(e) => {
                println!(
                    "
ERROR: failed to run test command!
  command: {:?}
  cwd: {}
  error: {}

Did you forget to build?",
                    &self.command.command, &self.command.cwd, e
                );
                std::process::exit(-1);
            }
        };
        let exit_code = output.status.code().unwrap_or(-7787);
        let stdout = std::str::from_utf8(&output.stdout).unwrap_or("couldn't decode output!");
        let stderr = std::str::from_utf8(&output.stderr).unwrap_or("couldn't decode output!");
        let output_str = stderr.to_owned() + stdout;
        self.cleanup(exit_code == 0, &output_paths)
            .expect("failed to clean up temporary output directory!");
        TestCommandResult {
            exit_code,
            stdout: output_str,
        }
    }

    fn cleanup(&self, _equal: bool, _output_paths: &OutputPaths) -> std::io::Result<()> {
        if let Some(tmp_path) = &self.command.tmp_path {
            if tmp_path.is_dir() && std::fs::read_dir(tmp_path).unwrap().next().is_none() {
                std::fs::remove_dir(&tmp_path)?;
            }
        }
        Ok(())
    }

    fn get_uid(&self) -> TestUid<'a> {
        (self.app_name, &self.test_id.id)
    }
}

fn test_apps_from_args(
    args: &clap::ArgMatches<'_>,
    input_paths: &config::InputPaths,
    test_group_file: &config::TestGroupFile,
) -> Vec<AppWithTests> {
    let id_filter = id_filter_from_args(args);
    let apps: Vec<AppWithTests> = args
        .values_of("test_app")
        .unwrap()
        .map(|app_name| {
            // get app for string from command line
            let app = match input_paths.apps.get(app_name) {
                Some(app) => app,
                None => {
                    let app_names = input_paths.apps.app_names();
                    println!(
                        "ERROR: \"{}\" not found: must be one of {:?}",
                        app_name, app_names
                    );
                    std::process::exit(-1);
                }
            };

            // populate with tests
            let empty_vec = vec![];
            let test_groups = test_group_file.get(app_name).unwrap_or(&empty_vec);
            AppWithTests {
                name: app_name.to_string(),
                app: (*app).clone(),
                tests: populate_test_groups(&app, input_paths, &test_groups, &*id_filter),
            }
        })
        .filter(|app_with_tests| !app_with_tests.tests.is_empty())
        .collect();
    if apps.is_empty() {
        println!("WARNING: you have not selected any tests.");
    }
    apps
}

fn id_filter_from_args(args: &clap::ArgMatches<'_>) -> Box<Fn(&str) -> bool> {
    let filter_tokens: Option<Vec<String>> = args
        .values_of("filter")
        .map(|v| v.map(String::from).collect());
    let normalize = |input: &str| input.to_lowercase().replace('\\', "/");
    let id_token = args.value_of("id").map(|v| normalize(v).to_string());
    Box::new(move |input: &str| {
        let input = normalize(input);
        if let Some(filters) = &filter_tokens {
            filters
                .iter()
                .any(|f| input.contains(normalize(f).as_str()))
        } else if let Some(id) = &id_token {
            input == *id
        } else {
            true
        }
    })
}

fn populate_test_groups(
    app: &config::App,
    input_paths: &config::InputPaths,
    test_groups: &[config::TestGroup],
    id_filter: &dyn Fn(&str) -> bool,
) -> Vec<GroupWithTests> {
    test_groups
        .iter()
        .map(|test_group| GroupWithTests {
            test_group: test_group.clone(),
            test_ids: test_group
                .generate_test_inputs(&app, &input_paths)
                .into_iter()
                .filter(|f| id_filter(&f.id))
                .collect(),
        })
        .collect()
}
