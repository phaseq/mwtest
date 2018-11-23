use std::collections::{hash_map, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

pub struct XmlReport {
    file: File,
    results: HashMap<String, Vec<(String, ::TestCommandResult)>>,
}
impl XmlReport {
    pub fn create(path: &Path) -> std::io::Result<XmlReport> {
        Ok(XmlReport {
            file: File::create(&path)?,
            results: HashMap::new(),
        })
    }
    pub fn add(&mut self, test_name: &str, test_id: &str, test_result: &::TestCommandResult) {
        match self.results.entry(test_name.to_string()) {
            hash_map::Entry::Vacant(e) => {
                e.insert(vec![(test_id.to_string(), test_result.clone())]);
            }
            hash_map::Entry::Occupied(mut e) => {
                e.get_mut().push((test_id.to_string(), test_result.clone()));
            }
        }
    }
    pub fn write(&mut self) -> std::io::Result<()> {
        let mut out = BufWriter::new(&self.file);
        out.write(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
        out.write(b"<testsuites>")?;
        for (test_name, test_results) in &self.results {
            out.write(
                format!(
                    "<testsuite name=\"{}\" test=\"{}\" failures=\"{}\">",
                    test_name,
                    test_results.len(),
                    -1
                ).as_bytes(),
            )?;
            for result in test_results.iter() {
                out.write(
                    format!(
                        "<testcase name=\"{}\">",
                        htmlescape::encode_attribute(&result.0)
                    ).as_bytes(),
                )?;
                out.write(format!("<exit-code>{}</exit_code>", result.1.exit_code).as_bytes())?;
                out.write(
                    format!(
                        "<system_out>{}</system_out>",
                        htmlescape::encode_minimal(&result.1.stdout)
                    ).as_bytes(),
                )?;
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
