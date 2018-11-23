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
    pub fn add(
        &self,
        i: usize,
        n: usize,
        test_name: &str,
        test_id: &str,
        test_result: &::TestCommandResult,
    ) {
        let (width, _) = term_size::dimensions().unwrap();
        if test_result.exit_code == 0 {
            if self.verbose {
                println!(
                    "[{}/{}] Ok: {} --id \"{}\"\n{}",
                    i, n, &test_name, &test_id, &test_result.stdout
                );
            } else {
                let line = format!("\r[{}/{}] Ok: {} --id \"{}\"", i, n, &test_name, &test_id);
                print!("{:width$}", line, width = width);
                std::io::stdout().flush().unwrap();
            }
        } else {
            println!(
                "[{}/{}] Failed: {} --id {}\n{}",
                i, n, &test_name, &test_id, &test_result.stdout
            );
        }
    }
}
impl Drop for StdOut {
    fn drop(&mut self) {
        if !self.verbose {
            println!();
        }
    }
}
