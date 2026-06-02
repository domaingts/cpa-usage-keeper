use std::{collections::HashMap, ops::{Add, Deref}};

struct MyMap(HashMap<String, String>);

fn v_static<T: Run>(s: T) {}

trait Run: Send + 'static {
    fn run(&self) -> String;
}

impl<'a> Deref for StaticTest<'a> {
    type Target = &'a str;

    fn deref(&self) -> &Self::Target {
        &self.name
    }
}

struct StaticTest<'a> {
    name: &'a str,
}

impl Run for StaticTest<'static> {
    fn run(&self) -> String {
        self.name.to_string()
    }
}

fn main() {
    {
        // let s: &str = "docker";

        // let d : &'static str = s;
        let s = String::from("docker");
        let d = s.as_str();
        let st = StaticTest { name: d };
        // v_static(st);
    }
}

// #[tokio::main]
// async fn main() -> ExitCode {
//     let env_file = parse_env_flag();

//     let mut application = match app::App::new_with_options(app::Options { env_file }).await {
//         Ok(app) => app,
//         Err(err) => {
//             eprintln!("initialize app: {err:#}");
//             return ExitCode::from(1);
//         }
//     };

//     let run_result = application.run().await;
//     if let Err(err) = application.close().await {
//         eprintln!("close app: {err:#}");
//     }
//     match run_result {
//         Ok(()) => ExitCode::SUCCESS,
//         Err(err) => {
//             eprintln!("run app: {err:#}");
//             ExitCode::from(1)
//         }
//     }
// }

// fn parse_env_flag() -> Option<PathBuf> {
//     let mut args = env::args().skip(1);
//     while let Some(arg) = args.next() {
//         if let Some(rest) = arg.strip_prefix("--env=") {
//             return Some(PathBuf::from(rest));
//         }
//         if arg == "--env" || arg == "-env" {
//             return args.next().map(PathBuf::from);
//         }
//         if let Some(rest) = arg.strip_prefix("-env=") {
//             return Some(PathBuf::from(rest));
//         }
//     }
//     None
// }
