use std::path::{Path, PathBuf};
use std::process::Command;

mod command_helpers;
use command_helpers::*;

pub struct Build {
    files: Vec<PathBuf>,
    classpath: Vec<PathBuf>,
    java_home: Option<PathBuf>,
    include_runtime: bool,
    no_jdk: bool,
    no_reflect: bool,
    no_stdlib: bool,
    warnings_into_errors: bool,
    cargo_output: CargoOutput,
}

impl Build {
    pub fn new() -> Self {
        Self {
            files: vec![],
            classpath: vec![],
            java_home: None,
            include_runtime: false,
            no_jdk: false,
            no_reflect: false,
            no_stdlib: false,
            warnings_into_errors: false,
            cargo_output: CargoOutput::new(),
        }
    }
    pub fn warnings_into_errors(&mut self, warnings_into_errors: bool) -> &mut Self {
        self.warnings_into_errors = warnings_into_errors;
        self
    }

    pub fn no_jdk(&mut self, no_jdk: bool) -> &mut Self {
        self.no_jdk = no_jdk;
        self
    }

    pub fn no_reflect(&mut self, no_reflect: bool) -> &mut Self {
        self.no_reflect = no_reflect;
        self
    }

    pub fn no_stdlib(&mut self, no_stdlib: bool) -> &mut Self {
        self.no_stdlib = no_stdlib;
        self
    }

    pub fn include_runtime(&mut self, include_runtime: bool) -> &mut Self {
        self.include_runtime = include_runtime;
        self
    }

    pub fn java_home<P: AsRef<Path>>(&mut self, p: P) -> &mut Self {
        self.java_home = Some(p.as_ref().into());
        self
    }

    pub fn file<P: AsRef<Path>>(&mut self, p: P) -> &mut Self {
        self.files.push(p.as_ref().into());
        self
    }

    pub fn classpath<P: AsRef<Path>>(&mut self, p: P) -> &mut Self {
        self.classpath.push(p.as_ref().into());
        self
    }

    pub fn classpaths<P>(&mut self, classpaths: P) -> &mut Self
    where
        P: IntoIterator,
        P::Item: AsRef<Path>,
    {
        for cp in classpaths {
            self.classpath(cp);
        }
        self
    }

    pub fn compile(&self, output: &str) -> Result<(), Error> {
        let mut cmd = Command::new("kotlinc-jvm");

        if !self.classpath.is_empty() {
            let classpath = self
                .classpath
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<String>>()
                .join(":");

            cmd.arg("-cp").arg(classpath);
        }

        if let Some(java_home) = &self.java_home {
            cmd.arg("-java-home").arg(java_home);
        }

        if self.include_runtime {
            cmd.arg("-include-runtime");
        }

        if self.no_jdk {
            cmd.arg("-no-jdk");
        }

        if self.no_reflect {
            cmd.arg("-no-reflect");
        }

        if self.no_stdlib {
            cmd.arg("-no-stdlib");
        }

        if self.warnings_into_errors {
            cmd.arg("-Werror");
        }

        for file in &self.files {
            cmd.arg(file);
        }

        cmd.arg("-d").arg(output);
        run(&mut cmd, "kotlinc-jvm", &self.cargo_output)
    }
}
