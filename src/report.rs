use crate::runnable;
use crate::scheduler;
use std::collections::{hash_map, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct Report {
    std_out: CliLogger,
    file_logger: FileLogger,
    xml_report: XmlReport,
}
impl Report {
    pub fn new(artifacts_root: &Path, testcases_root: &str, verbose: bool) -> Report {
        let xml_location = &artifacts_root.join("results.xml");
        let report = Report {
            std_out: CliLogger::create(verbose),
            file_logger: FileLogger::new(&artifacts_root),
            xml_report: XmlReport::create(&xml_location, &artifacts_root, &testcases_root).unwrap(),
        };
        report.std_out.init();
        report
    }

    pub fn add(
        &mut self,
        i: usize,
        n: usize,
        test_instance: runnable::TestInstance,
        test_result: &scheduler::TestCommandResult,
    ) {
        self.std_out.add(
            i,
            n,
            &test_instance.app_name,
            &test_instance.test_id.id,
            &test_result,
        );
        self.file_logger
            .add(&test_instance.app_name, &test_result.stdout);
        self.xml_report.add(test_instance, &test_result);
    }
}

struct XmlReport {
    file: File,
    results: HashMap<String, Vec<(runnable::TestInstance, scheduler::TestCommandResult)>>,
    artifacts_root: PathBuf,
    testcases_root: PathBuf,
}
impl XmlReport {
    fn create(
        path: &Path,
        artifacts_root: &Path,
        testcases_root: &str,
    ) -> std::io::Result<XmlReport> {
        Ok(XmlReport {
            file: File::create(&path)?,
            results: HashMap::new(),
            artifacts_root: PathBuf::from(artifacts_root),
            testcases_root: PathBuf::from(testcases_root),
        })
    }

    fn add(
        &mut self,
        test_instance: runnable::TestInstance,
        test_result: &scheduler::TestCommandResult,
    ) {
        match self.results.entry(test_instance.app_name.to_string()) {
            hash_map::Entry::Vacant(e) => {
                e.insert(vec![(test_instance, test_result.clone())]);
            }
            hash_map::Entry::Occupied(mut e) => {
                e.get_mut().push((test_instance, test_result.clone()));
            }
        }
    }

    fn write(&mut self) -> std::io::Result<()> {
        let mut out = BufWriter::new(&self.file);
        out.write_all(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><mwtest>")?;
        out.write_all(
            format!(
                "<config><reference_root>{}</reference_root></config>",
                self.testcases_root.to_string_lossy().into_owned()
            )
            .as_bytes(),
        )?;
        out.write_all(b"<testsuites>")?;
        for (test_name, test_results) in &self.results {
            out.write_all(
                format!(
                    "<testsuite name=\"{}\" test=\"{}\">",
                    test_name,
                    test_results.len()
                )
                .as_bytes(),
            )?;
            for (test_instance, command_result) in test_results.iter() {
                self.write_testcase(&mut out, &test_instance, &command_result)?;
            }
            out.write_all(b"</testsuite>")?;
        }
        out.write_all(b"</testsuites></mwtest>")?;
        Ok(())
    }

    fn write_testcase(
        &self,
        out: &mut BufWriter<&File>,
        test_instance: &runnable::TestInstance,
        command_result: &scheduler::TestCommandResult,
    ) -> std::io::Result<()> {
        out.write_all(
            format!(
                "<testcase name=\"{}\">",
                htmlescape::encode_attribute(&test_instance.test_id.id)
            )
            .as_bytes(),
        )?;
        out.write_all(format!("<exit_code>{}</exit_code>", command_result.exit_code).as_bytes())?;
        if command_result.exit_code != 0 {
            out.write_all(b"<failure />")?;
        }
        out.write_all(
            format!(
                "<system_out>{}</system_out>",
                htmlescape::encode_minimal(&command_result.stdout)
            )
            .as_bytes(),
        )?;
        if let Some(tmp_path) = &test_instance.command.tmp_path {
            if tmp_path.exists() {
                let rel_path = test_instance.test_id.rel_path.as_ref().unwrap();
                let abs_reference_path = self.testcases_root.join(rel_path);
                let sub_dir = if command_result.exit_code == 0 {
                    "success"
                } else {
                    "different"
                };
                let mut abs_artifact_path = self.artifacts_root.join(sub_dir).join(rel_path);
                if abs_artifact_path.exists() {
                    abs_artifact_path.set_file_name(
                        abs_artifact_path
                            .file_name()
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .to_string()
                            + &Uuid::new_v4().to_string(),
                    );
                }

                if abs_reference_path.is_dir() || tmp_path.is_file() {
                    std::fs::create_dir_all(abs_artifact_path.parent().unwrap())?;
                    std::fs::rename(&tmp_path, &abs_artifact_path)?;
                    self.write_artifact(out, &abs_reference_path, &abs_artifact_path)?;
                } else {
                    let abs_artifact_dir = abs_artifact_path.parent().unwrap();
                    std::fs::create_dir_all(&abs_artifact_dir)?;
                    for entry in std::fs::read_dir(tmp_path)? {
                        let from = entry?.path();
                        let file_name = &from.file_name().unwrap();
                        let to = abs_artifact_dir.join(file_name);
                        std::fs::rename(&from, &to)?;
                        self.write_artifact(out, &abs_reference_path, &abs_artifact_dir)?;
                    }
                }
            }
        }
        out.write_all(b"</testcase>")?;
        Ok(())
    }

    fn write_artifact(
        &self,
        out: &mut BufWriter<&File>,
        abs_reference_path: &Path,
        abs_artifact_path: &Path,
    ) -> std::io::Result<()> {
        let rel_reference_path = abs_reference_path
            .strip_prefix(&self.testcases_root)
            .unwrap()
            .to_str()
            .unwrap();
        let rel_artifact_path = abs_artifact_path
            .strip_prefix(&self.artifacts_root)
            .unwrap()
            .to_str()
            .unwrap();
        out.write_all(
            format!(
                "<artifact reference=\"{}\" location=\"{}\" />",
                htmlescape::encode_attribute(&rel_reference_path),
                htmlescape::encode_attribute(&rel_artifact_path)
            )
            .as_bytes(),
        )?;
        Ok(())
    }
}
impl<'a> Drop for XmlReport {
    fn drop(&mut self) {
        self.write().expect("failed to write xml log!");
    }
}

struct CliLogger {
    verbose: bool,
    term_width: Option<usize>,
    run_counts: HashMap<TestUid, RunCount>,
}
type TestUid = (String, String);
struct RunCount {
    n_runs: u32,
    n_successes: u32,
}
impl CliLogger {
    fn create(verbose: bool) -> CliLogger {
        CliLogger {
            verbose,
            term_width: term_size::dimensions_stdout().map(|(w, _h)| w),
            run_counts: HashMap::new(),
        }
    }
    fn init(&self) {
        print!("waiting for results...");
        std::io::stdout().flush().unwrap();
    }
    fn add(
        &mut self,
        i: usize,
        n: usize,
        name: &str,
        id: &str,
        result: &scheduler::TestCommandResult,
    ) {
        // generate progress message
        let ok_or_failed = if result.exit_code == 0 {
            "Ok"
        } else {
            "Failed"
        };
        let mut line = format!("[{}/{}] {}: {} --id \"{}\"", i, n, ok_or_failed, &name, &id);

        // keep replacing OK message (only) when printing to a TTY
        if let Some(width) = self.term_width {
            line.truncate(width);
            print!("\r{:width$}", line, width = width);
        } else {
            println!("{}", line);
        }

        // print full test output if requested
        if result.exit_code != 0 || self.verbose {
            println!("\n{}\n", &result.stdout.trim());
        }

        // flush if a TTY is attached
        if self.term_width.is_some() {
            std::io::stdout().flush().unwrap();
        }

        let entry = self
            .run_counts
            .entry((name.to_string(), id.to_string()))
            .or_insert(RunCount {
                n_runs: 0,
                n_successes: 0,
            });
        entry.n_runs += 1;
        if result.exit_code == 0 {
            entry.n_successes += 1;
        }
    }

    fn report_summary(&self) -> bool {
        let test_formatter = |(id, run_counts): (&TestUid, &RunCount)| {
            if run_counts.n_runs > 1 {
                format!(
                    "  {} --id \"{}\" (succeeded {} out of {} runs)",
                    id.0, id.1, run_counts.n_successes, run_counts.n_runs
                )
            } else {
                format!("  {} --id \"{}\"", id.0, id.1)
            }
        };
        let mut failed: Vec<String> = self
            .run_counts
            .iter()
            .filter(|(_id, run_counts)| run_counts.n_successes == 0)
            .map(test_formatter)
            .collect();
        failed.sort_unstable();
        let all_succeeded = failed.is_empty();

        let mut instable: Vec<String> = self
            .run_counts
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
}
impl Drop for CliLogger {
    fn drop(&mut self) {
        if !self.verbose {
            println!();
        }
        self.report_summary();
    }
}

struct FileLogger {
    log_dir: PathBuf,
    files: HashMap<String, File>,
}
impl FileLogger {
    fn new(log_dir: &Path) -> FileLogger {
        FileLogger {
            log_dir: log_dir.to_path_buf(),
            files: HashMap::new(),
        }
    }
    fn add(&mut self, test_name: &str, output: &str) {
        let log_dir = &self.log_dir;
        let log_file = self.files.entry(test_name.to_owned()).or_insert_with(|| {
            File::create(log_dir.join(PathBuf::from(test_name.to_owned() + ".txt")))
                .expect("could not create log file!")
        });
        log_file
            .write_all(output.as_bytes())
            .expect("could not write to log file!");
    }
}
