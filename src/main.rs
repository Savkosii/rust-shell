use std::path::PathBuf;
use std::io;
use std::process::{Command, Child, Stdio};
use std::fs::File;
use std::io::Write;
use std::char;
use glob::glob;
use dirs::home_dir;
use whoami;

fn catch_sytax_error(line: &str) -> Option<String> {
    let line = line.trim();
    let tokens = vec![";", "|", "&&", ">>", "<<", ">", "<"];
    for &token in &tokens[..3] {
        if line.starts_with(token) {
            return Some(format!("shell: syntax error near unexpected token '{}'", token));
        }
    }

    let mut pos: Vec<(usize, &str)> = Vec::new();
    let mut prev_pos = line.len();
    for &token in &tokens {
        for pair in line.match_indices(token) {
            if !line.index_in_escape_scope(pair.0) {
                if token == ">" || token == "<" {
                    if pair.0 == prev_pos + 1 {
                        pos.pop();
                        prev_pos = line.len();
                        continue;
                    } else {
                        prev_pos = pair.0;
                    }
                }
                pos.push(pair);
            }
        }
    }

    pos.sort_by(|m, n| m.0.cmp(&n.0));
    let mut prev = 0;
    for (index, token) in pos {
        if line[prev..index].trim().is_empty() {
            eprintln!("shell: syntax error near unexpected token '{}'", token);
            return Some(format!("shell: syntax error near unexpected token '{}'", token));
        }
        prev = index + token.len();
    }
    None
}

 fn load_command_line(buf: &mut String) -> Result<usize, String> {
    let nbytes = io::stdin().read_line(buf).unwrap();

    let v : Vec<_> = buf.matches("\"").collect();
    if v.len() % 2 != 0 {
        return load_command_line(buf);
    }

    if let Some(e) = catch_sytax_error(&buf) {
        return Err(e);
    }

    let tokens = vec![";", "|", "&&", ">>", "<<", ">", "<"];
    let line = buf.trim_end();

    for token in &tokens[1..] {
        if line.ends_with(token) {
            return load_command_line(buf);
        }
    }

    Ok(nbytes)
 }

trait Split {
    fn index_in_escape_scope(&self, index: usize) -> bool ;

    fn split_with_strs<'a>(&'a self, token: &[&str]) -> Vec<&'a str> ;

    fn split_with_chars<'a, F>(&'a self, f: F ) -> Vec<&'a str> 
        where F: Fn(char) -> bool;
}

impl Split for str {
    fn index_in_escape_scope(&self, index: usize) -> bool {
        let pos: Vec<usize> = self.match_indices("\"").map(|x| x.0 ).collect();
        let mut escape_scope: Vec<(usize, usize)> = Vec::new();
        for pair in pos.chunks(2) {
            escape_scope.push((pair[0] + 1, pair[1]));
        }
        for (left, right) in escape_scope {
            if index >= left && index < right {
                return true
            }
        }
        false
    }

    fn split_with_strs<'a>(&'a self, tokens: &[&str]) -> Vec<&'a str> {
        let mut breakpoints: Vec<(usize, &str)> = Vec::new();
        for token in tokens {
            let mut search: Vec<(usize, &str)> = self.match_indices(token)
                                    .filter(|x| !self.index_in_escape_scope(x.0))
                                    .collect();
            breakpoints.append(&mut search);
        }

        breakpoints.sort_by(|x, y| x.0.cmp(&y.0));

        let mut prev = 0;
        let mut res: Vec<&str> = Vec::new();
        for (i, token) in breakpoints {
            res.push(&self[prev..i]);
            prev = i + token.len();
        }
        if prev < self.len() {
            res.push(&self[prev..]);
        }
        res
    }

    fn split_with_chars<'a, F> (&'a self, is_token: F) -> Vec<&'a str> 
        where F: Fn(char) -> bool {
        let pos: Vec<usize> = self.match_indices(is_token)
                                    .map(|x| x.0)
                                    .filter(|&i| !self.index_in_escape_scope(i))
                                    .collect();
        let mut breakpoints: Vec<usize> = Vec::new();
        for i in 0 .. pos.len() {
            if i == 0 || pos[i] != pos[i-1] + 1 {
                breakpoints.push(pos[i]);
            }
            if i == pos.len() - 1 || pos[i] != pos[i+1] - 1 {
                breakpoints.push(pos[i]);
            }
        }
        let mut prev = 0;
        let mut res: Vec<&str> = Vec::new();
        for pair in breakpoints.chunks(2) {
            res.push(&self[prev..pair[0]]);
            prev = pair[1] + 1;
        }
        if prev < self.len() {
            res.push(&self[prev..]);
        }
        res
    }
}


fn parse_command<'a> (line: &'a str) -> Vec<&'a str> {
    line.trim().split_with_strs(&[";", "&&"])
}

fn parse_argv<'a> (command: &str) -> Vec<String> {
    let argv = command.trim().split_with_chars(char::is_whitespace);
    let mut real_argv: Vec<String> = Vec::new();
    for arg in argv {
        for real_arg in arg.unfold().match_wild_card() {
            real_argv.push(real_arg);
        }
    }
    real_argv
}

trait PathMatcher {
    fn match_wild_card(&self) -> Vec<String>;
    fn unfold(&self) -> String;
}

impl PathMatcher for str {
    fn match_wild_card(&self) -> Vec<String> {
        let mut res: Vec<String> = Vec::new();
        for path in glob(self).unwrap() {
            res.push(path.unwrap().to_str().unwrap().to_owned());
        }
        if res.is_empty() {
            res.push(self.to_owned());
        }
        res
    }
    fn unfold(&self) -> String {
        if self == "~" {
            home_dir().unwrap().to_str().unwrap().to_owned()
        } else if self.starts_with("~/") {
            home_dir().unwrap().join(PathBuf::from(&self[2..]))
            .to_str().unwrap().to_owned()
        } else {
            self.to_owned()
        }
    }
}

fn parse_file_path(path: &str) -> Option<String> {
    let res = path.unfold().match_wild_card();
    if res.len() > 1 {
        eprint!("shell: {}: ambiguous redirect", path);
        None
    } else {
        Some(res[0].to_owned())
    }
}

trait Wrapper {
    fn locate_file_stream(argv: &mut Vec<String>) -> Option<File>;
    fn apply_file_stream_filter(&mut self, resources: Option<File>) -> &mut Self;
    fn apply_pipe_stream_filter(&mut self, prev_command: &mut Option<Child>, 
                                istream: bool, wstream: bool) -> &mut Self;
}

impl Wrapper for Command {
    fn locate_file_stream(argv: &mut Vec<String>) -> Option<File> {
        let mut stream: Option<File> = None;
        let mut flag = 0;
        let mut real_argv: Vec<String> = Vec::new();
        for arg in argv.iter() {
            if arg.starts_with(">>") {
                if arg == ">>" {
                    flag = 1;
                } else {
                    stream = File::options()
                            .create(true)
                            .append(true)
                            .open(parse_file_path(&arg[2..])?)
                            .map_or_else(
                                |e| {
                                    eprintln!("{}", e);
                                    None
                                },
                                |v| {
                                    Some(v)
                                }
                            );
                }
            } else if arg.starts_with(">") {
                if arg == ">" {
                    flag = -1;
                } else {
                    stream = File::options()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(parse_file_path(&arg[1..])?)
                            .map_or_else(
                                |e| {
                                    eprintln!("{}", e);
                                    None
                                },
                                |v| {
                                    Some(v)
                                }
                            );
                }
            } else {
                let mut real_arg = arg.as_str();
                if arg.starts_with("\"") {
                    real_arg = &arg[1..arg.len()-1];
                }
                match flag {
                    1 => stream = File::options()
                                .create(true)
                                .append(true)
                                .open(parse_file_path(real_arg)?)
                                .map_or_else(
                                |e| {
                                    eprintln!("{}", e);
                                    None
                                },
                                |v| {
                                    Some(v)
                                }
                            ),

                    -1 => stream = File::options()
                                .create(true)
                                .write(true)
                                .truncate(true)
                                .open(parse_file_path(real_arg)?)
                                .map_or_else(
                                    |e| {
                                        eprintln!("{}", e);
                                        None
                                    },
                                    |v| {
                                        Some(v)
                                    }
                                ),
                    _ => {
                        flag = 0;
                        real_argv.push(real_arg.to_owned());
                    }
                }
            }
        }
        *argv = real_argv;
        stream
    }

    fn apply_file_stream_filter(&mut self, resources: Option<File>) -> &mut Self {
        if let Some(stream) = resources {
            self.stdout(stream)
        } else {
            self
        }
    }

    fn apply_pipe_stream_filter(mut self: &mut Self, 
        prev_command: &mut Option<std::process::Child>, istream: bool, wstream: bool) 
    -> &mut Self {
        if wstream {
            self = self.stdout(Stdio::piped());
        }
        if istream {
            if let Some(x) = prev_command.as_mut().unwrap().stdout.take() {
                self = self.stdin(x);
            }
        }
        self
    }
}


fn chdir(argv: &[String]) {
    if argv.len() > 2 {
        eprintln!("shell: cd: too many arguments");
        return;
    }
    let path = if argv.len() == 1 {
        home_dir().unwrap()
    } else {
        PathBuf::from(&argv[1])
    };
    std::env::set_current_dir(&path).unwrap_or_else(|e| eprintln!("{}", e));
}


fn exec_command_with_pipes(line: &str) -> Option<std::process::Child> {
    let commands = line.trim().split_with_strs(&["|"]);
    let mut prev_command: Option<std::process::Child> = None;
    let mut commands_count = 0;
    let mut commands_nums = commands.len();
    for i in 0..commands.len() {
        let mut argv = parse_argv(commands[i].trim());
        let resources = Command::locate_file_stream(&mut argv);
        let argv_option = match argv.len() {
            1 => &[],
            _ => &argv[1..],
        };
        if argv[0] == "cd" {
            chdir(&argv);
            commands_nums -= 1;
            continue;
        };
        if commands_count == 0 {
            prev_command = Command::new(&argv[0])
                            .args(argv_option)
                            .apply_pipe_stream_filter(&mut prev_command, false, true)
                            .apply_file_stream_filter(resources)
                            .spawn()
                            .map_or_else(
                                |e| {
                                    eprintln!("{}", e); 
                                    None
                                }, 
                                |v| {
                                    Some(v)
                                }
                            )

        } else if commands_count != commands_nums - 1 {
            prev_command = Command::new(&argv[0])
                            .args(argv_option)
                            .apply_pipe_stream_filter(&mut prev_command, true, true)
                            .apply_file_stream_filter(resources)
                            .spawn()
                            .map_or_else(
                                |e| {
                                    eprintln!("{}", e); 
                                    None
                                }, 
                                |v| {
                                    Some(v)
                                }
                            )
        } else {
            prev_command = Command::new(&argv[0])
                            .args(argv_option)
                            .apply_pipe_stream_filter(&mut prev_command, true, false)
                            .apply_file_stream_filter(resources)
                            .spawn()
                            .map_or_else(
                                |e| {
                                    eprintln!("{}", e); 
                                    None
                                }, 
                                |v| {
                                    Some(v)
                                }
                            )
        }
        commands_count += 1;
    }
    prev_command
}

fn exec_normal_command(command: &str) -> Option<std::process::Child> {
    let mut argv = parse_argv(command.trim());
    let resources = Command::locate_file_stream(&mut argv);
    let argv_option = match argv.len() {
        1 => &[],
        _ => &argv[1..],
    };
    if argv[0] == "cd" {
        chdir(&argv);
        return None;
    }
    Command::new(&argv[0])
            .args(argv_option)
            .apply_file_stream_filter(resources)
            .spawn()
            .map_or_else(
                |e| {
                    eprintln!("{}", e); 
                    None
                }, 
                |v| {
                    Some(v)
                }
            )
}

fn exec_commands(line: &str) {
    let commands = parse_command(line);
    for command in commands {
        let last_command = match command.find("|") {
            Some(_) => exec_command_with_pipes(line),
            _ => exec_normal_command(line)
        };
        if let Some(mut cmd) = last_command {
            cmd.wait().unwrap();
        }
    }
}

fn prompt() {
    let username = whoami::username();
    let hostname = whoami::hostname();
    let home_dir = String::from(home_dir().unwrap().to_str().unwrap());
    let current_dir = String::from(std::env::current_dir().unwrap().to_str().unwrap());
    let prompt_path = if current_dir.starts_with(&home_dir) {
        if current_dir.len() == home_dir.len() {
            String::from("~")
        } else if current_dir[home_dir.len()..].starts_with("/") {
            format!("~{}", &current_dir[home_dir.len()..])
        } else {
            current_dir
        }
    } else {
        current_dir
    };
    let ch = match username == "root" {
        true => '#',
        false => '$'
    };
    print!("{}@{}:{}{} ", username, hostname, prompt_path, ch);
    std::io::stdout().flush().unwrap();
}


fn main() {
    loop {
        prompt();
        let mut s = String::new();
        match load_command_line(&mut s) {
            Ok(n) => {
                //EOF
                if n == 0 {
                    return;
                }
                exec_commands(&s);
            }
            Err(e) => eprintln!("{}", e),
        }
    }
}
