use std::collections::{hash_map, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct Report<'a> {
    std_out: CliLogger,
    file_logger: FileLogger<'a>,
    xml_report: XmlReport<'a>,
}
impl<'a> Report<'a> {
    pub fn new(artifacts_root: &'a Path, testcases_root: &str, verbose: bool) -> Report<'a> {
        let xml_location = &artifacts_root.join("results.xml");
        let report = Report {
            std_out: CliLogger { verbose: verbose },
            file_logger: FileLogger::new(&artifacts_root),
            xml_report: XmlReport::new(&xml_location, &artifacts_root, &testcases_root).unwrap(),
        };
        report.std_out.new();
        report
    }

    pub fn add(
        &mut self,
        i: usize,
        n: usize,
        test_instance: ::TestInstance<'a>,
        test_result: &::TestCommandResult,
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

struct XmlReport<'a> {
    file: File,
    results: HashMap<String, Vec<(::TestInstance<'a>, ::TestCommandResult)>>,
    artifacts_root: PathBuf,
    testcases_root: PathBuf,
}
impl<'a> XmlReport<'a> {
    fn new(
        path: &Path,
        artifacts_root: &Path,
        testcases_root: &str,
    ) -> std::io::Result<XmlReport<'a>> {
        Ok(XmlReport {
            file: File::create(&path)?,
            results: HashMap::new(),
            artifacts_root: PathBuf::from(artifacts_root),
            testcases_root: PathBuf::from(testcases_root),
        })
    }

    fn add(&mut self, test_instance: ::TestInstance<'a>, test_result: &::TestCommandResult) {
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
        out.write(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><mwtest>")?;
        out.write(
            format!(
                "<config><reference_root>{}</reference_root></config>",
                self.testcases_root.to_string_lossy().into_owned()
            ).as_bytes(),
        )?;
        out.write(b"<testsuites>")?;
        for (test_name, test_results) in &self.results {
            out.write(
                format!(
                    "<testsuite name=\"{}\" test=\"{}\">",
                    test_name,
                    test_results.len()
                ).as_bytes(),
            )?;
            for (test_instance, command_result) in test_results.iter() {
                self.write_testcase(&mut out, &test_instance, &command_result)?;
            }
            out.write(b"</testsuite>")?;
        }
        out.write(b"</testsuites></mwtest>")?;
        Ok(())
    }

    fn write_testcase(
        &self,
        out: &mut BufWriter<&File>,
        test_instance: &::TestInstance<'a>,
        command_result: &::TestCommandResult,
    ) -> std::io::Result<()> {
        out.write(
            format!(
                "<testcase name=\"{}\">",
                htmlescape::encode_attribute(&test_instance.test_id.id)
            ).as_bytes(),
        )?;
        out.write(format!("<exit_code>{}</exit_code>", command_result.exit_code).as_bytes())?;
        if command_result.exit_code != 0 {
            out.write(b"<failure />")?;
        }
        out.write(
            format!(
                "<system_out>{}</system_out>",
                htmlescape::encode_minimal(&command_result.stdout)
            ).as_bytes(),
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
                let abs_artifact_path = self.artifacts_root.join(sub_dir).join(rel_path);
                if abs_reference_path.is_dir() || tmp_path.is_file() {
                    std::fs::create_dir_all(abs_artifact_path.parent().unwrap())?;
                    std::fs::rename(&tmp_path, &abs_artifact_path)?;
                    self.write_artifact(out, &abs_artifact_path, &abs_artifact_path)?;
                } else {
                    std::fs::create_dir_all(&abs_artifact_path)?;
                    for entry in std::fs::read_dir(tmp_path)? {
                        let from = entry?.path();
                        let file_name = &from.file_name().unwrap();
                        let to = abs_reference_path.join(file_name);
                        std::fs::rename(&from, &to)?;
                        self.write_artifact(out, &abs_artifact_path, &abs_artifact_path)?;
                    }
                }
            }
        }
        out.write(b"</testcase>")?;
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
        out.write(
            format!(
                "<artifact reference=\"{}\" location=\"{}\" />",
                htmlescape::encode_attribute(&rel_reference_path),
                htmlescape::encode_attribute(&rel_artifact_path)
            ).as_bytes(),
        )?;
        Ok(())
    }
}
impl<'a> Drop for XmlReport<'a> {
    fn drop(&mut self) {
        self.write().expect("failed to write xml log!");
    }
}

struct CliLogger {
    verbose: bool,
}
impl CliLogger {
    fn new(&self) {
        print!("waiting for results...");
        std::io::stdout().flush().unwrap();
    }
    fn add(&self, i: usize, n: usize, name: &str, id: &str, result: &::TestCommandResult) {
        let ok_or_failed = if result.exit_code == 0 {
            "Ok"
        } else {
            "Failed"
        };
        let mut line = format!(
            "\r[{}/{}] {}: {} --id \"{}\"",
            i, n, ok_or_failed, &name, &id
        );
        let (width, _) = term_size::dimensions().unwrap();
        line.truncate(width);
        print!("{:width$}", line, width = width);
        if result.exit_code != 0 || self.verbose {
            println!("\n{}\n", &result.stdout.trim());
        }
        std::io::stdout().flush().unwrap();
    }
}
impl Drop for CliLogger {
    fn drop(&mut self) {
        if !self.verbose {
            println!();
        }
    }
}

struct FileLogger<'a> {
    log_dir: PathBuf,
    files: HashMap<&'a str, File>,
}
impl<'a> FileLogger<'a> {
    fn new(log_dir: &Path) -> FileLogger {
        FileLogger {
            log_dir: log_dir.to_path_buf(),
            files: HashMap::new(),
        }
    }
    fn add(&mut self, test_name: &'a str, output: &str) {
        let log_dir = &self.log_dir;
        let log_file = self.files.entry(test_name).or_insert_with(|| {
            File::create(log_dir.join(PathBuf::from(test_name.to_owned() + ".txt")))
                .expect("could not create log file!")
        });
        log_file
            .write(output.as_bytes())
            .expect("could not write to log file!");
    }
}
