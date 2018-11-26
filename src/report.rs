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
            xml_report: XmlReport::new(&xml_location, &testcases_root).unwrap(),
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
    file_path: PathBuf,
    results: HashMap<String, Vec<(::TestInstance<'a>, ::TestCommandResult)>>,
    testcases_root: String,
}
impl<'a> XmlReport<'a> {
    fn new(path: &Path, testcases_root: &str) -> std::io::Result<XmlReport<'a>> {
        Ok(XmlReport {
            file: File::create(&path)?,
            file_path: PathBuf::from(path),
            results: HashMap::new(),
            testcases_root: testcases_root.to_string(),
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
                self.testcases_root
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
                out.write(
                    format!(
                        "<testcase name=\"{}\">",
                        htmlescape::encode_attribute(&test_instance.test_id.id)
                    ).as_bytes(),
                )?;
                out.write(
                    format!("<exit_code>{}</exit_code>", command_result.exit_code).as_bytes(),
                )?;
                if command_result.exit_code != 0 {
                    out.write(b"<failure />")?;
                }
                out.write(
                    format!(
                        "<system_out>{}</system_out>",
                        htmlescape::encode_minimal(&command_result.stdout)
                    ).as_bytes(),
                )?;
                if let ::TmpPath::None = &test_instance.command.tmp_path {
                } else {
                    let tmp_dir = match &test_instance.command.tmp_path {
                        ::TmpPath::File(tmp_dir) => tmp_dir,
                        ::TmpPath::Dir(tmp_dir) => tmp_dir,
                        _ => panic!(),
                    };
                    if PathBuf::from(tmp_dir).exists() {
                        let rel_tmp_dir = PathBuf::from(tmp_dir);
                        let rel_tmp_dir = rel_tmp_dir
                            .strip_prefix(self.file_path.parent().unwrap())
                            .unwrap();
                        let rel_tmp_dir = rel_tmp_dir.to_string_lossy().into_owned();
                        let rel_reference_path = match &test_instance.test_id.rel_path {
                            ::RelTestLocation::Dir(p) => p,
                            ::RelTestLocation::File(p) => p,
                            _ => panic!(""),
                        }.to_string_lossy()
                        .into_owned();
                        out.write(
                            format!(
                                "<artifact reference=\"{}\" location=\"{}\" />",
                                htmlescape::encode_attribute(&rel_reference_path),
                                htmlescape::encode_attribute(&rel_tmp_dir)
                            ).as_bytes(),
                        )?;
                    }
                }
                out.write(b"</testcase>")?;
            }
            out.write(b"</testsuite>")?;
        }
        out.write(b"</testsuites></mwtest>")?;
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
