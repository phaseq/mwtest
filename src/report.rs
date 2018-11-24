use std::collections::{hash_map, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct XmlReport<'a> {
    file: File,
    file_path: PathBuf,
    results: HashMap<String, Vec<(::TestInstance<'a>, ::TestCommandResult)>>,
    testcases_root: String,
}
impl<'a> XmlReport<'a> {
    pub fn create(path: &Path, testcases_root: &str) -> std::io::Result<XmlReport<'a>> {
        Ok(XmlReport {
            file: File::create(&path)?,
            file_path: PathBuf::from(path),
            results: HashMap::new(),
            testcases_root: testcases_root.to_string(),
        })
    }
    pub fn add(&mut self, test_instance: ::TestInstance<'a>, test_result: &::TestCommandResult) {
        match self.results.entry(test_instance.app_name.to_string()) {
            hash_map::Entry::Vacant(e) => {
                e.insert(vec![(test_instance, test_result.clone())]);
            }
            hash_map::Entry::Occupied(mut e) => {
                e.get_mut().push((test_instance, test_result.clone()));
            }
        }
    }
    pub fn write(&mut self) -> std::io::Result<()> {
        let mut out = BufWriter::new(&self.file);
        out.write(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
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
            for result in test_results.iter() {
                out.write(
                    format!(
                        "<testcase name=\"{}\">",
                        htmlescape::encode_attribute(&result.0.test_id.id)
                    ).as_bytes(),
                )?;
                out.write(format!("<exit-code>{}</exit_code>", result.1.exit_code).as_bytes())?;
                out.write(
                    format!(
                        "<system_out>{}</system_out>",
                        htmlescape::encode_minimal(&result.1.stdout)
                    ).as_bytes(),
                )?;
                if let Some(tmp_dir) = &result.0.command.tmp_dir {
                    if PathBuf::from(tmp_dir).exists() {
                        let rel_tmp_dir = PathBuf::from(tmp_dir);
                        let rel_tmp_dir = rel_tmp_dir
                            .strip_prefix(self.file_path.parent().unwrap())
                            .unwrap();
                        let rel_tmp_dir = rel_tmp_dir.to_string_lossy().into_owned();
                        let rel_reference_path = match &result.0.test_id.rel_path {
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
            out.write(b"</testuite>")?;
        }
        out.write(b"</testsuites>")?;
        Ok(())
    }
}

pub struct StdOut {
    pub verbose: bool,
}
impl StdOut {
    pub fn init(&self, i: usize, n: usize) {
        print!("[{}/{}] waiting for results...", i, n);
        std::io::stdout().flush().unwrap();
    }
    pub fn add(&self, i: usize, n: usize, name: &str, id: &str, result: &::TestCommandResult) {
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
            println!("\n{}\n", &result.stdout);
        }
        std::io::stdout().flush().unwrap();
    }
}
impl Drop for StdOut {
    fn drop(&mut self) {
        if !self.verbose {
            println!();
        }
    }
}

pub struct FileLogger<'a> {
    log_dir: PathBuf,
    files: HashMap<&'a str, File>,
}
impl<'a> FileLogger<'a> {
    pub fn create(log_dir: &Path) -> FileLogger {
        FileLogger {
            log_dir: log_dir.to_path_buf(),
            files: HashMap::new(),
        }
    }
    pub fn add(&mut self, test_name: &'a str, output: &str) {
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
