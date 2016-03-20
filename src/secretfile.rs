//! Map application-level credential names to secrets in the backend store.
//!
//! In the case of Vault, this is necessary to transform
//! environment-variable-style credential names into Vault secret paths and
//! keys: from `MY_SECRET_PASSWORD` to the path `secret/my_secret` and the
//! key `"password"`.

use backend::{BoxedError, err};
use regex::{Captures, Regex};
use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{self, BufRead};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    // We'll use this for Keywhiz and other systems which store simple
    // string credentials.
    //Simple(String),
    /// We use this for systems like Vault which store key-value
    /// dictionaries in each secret.
    Keyed(String, String),
}

/// Interpolate environment variables into a string.
fn interpolate_env_vars(text: &str) -> Result<String, BoxedError> {
    // Only compile this Regex once.
    lazy_static! {
        static ref RE: Regex =
            Regex::new(r"\$(?:(?P<name>[a-zA-Z_][a-zA-Z0-9_]*)|\{(?P<name2>[a-zA-Z_][a-zA-Z0-9_]*)\})").unwrap();
    }

    // Perform the replacement.  This is mostly error-handling logic,
    // because `replace_all` doesn't anticipate any errors.
    let mut undefined_env_var = None;
    let result = RE.replace_all(text, |caps: &Captures| {
        let name =
            caps.name("name").or_else(|| { caps.name("name2") }).unwrap();
        match env::var(name) {
            Ok(s) => s.to_owned(),
            Err(_) => {
                undefined_env_var = Some(name.to_owned());
                "".to_owned()
            }
        }
    });
    match undefined_env_var {
        None => Ok(result),
        Some(var) => {
            let msg =
                format!("Secretfile: Environment variable {} is not defined",
                        var);
            Err(err(msg))
        }
    }
}

#[derive(Debug, Clone)]
pub struct Secretfile {
    mappings: BTreeMap<String, Location>,
}

impl Secretfile {
    /// Read in from an `io::Read` object.
    pub fn read(read: &mut io::Read) -> Result<Secretfile, BoxedError> {
        // Match a line of our file.
        let re = Regex::new(r"(?x)
^(?:
   # Blank line with optional comment.
   \s*(?:\#.*)?
 |
   # NAME path/to/secret:key
   (?P<name>\S+)
   \s+
   (?P<path>\S+?):(?P<key>\S+)
   \s*
 )$").unwrap();

        // TODO: Environment interpolation.
        let mut sf = Secretfile { mappings: BTreeMap::new() };
        let buffer = io::BufReader::new(read);
        for line_or_err in buffer.lines() {
            let line = try!(line_or_err);
            match re.captures(&line) {
                Some(ref caps) if caps.name("name").is_some() => {
                    let location = Location::Keyed(
                        try!(interpolate_env_vars(caps.name("path").unwrap())),
                        caps.name("key").unwrap().to_owned(),
                    );
                    sf.mappings.insert(caps.name("name").unwrap().to_owned(),
                                       location);
                }
                Some(_) => { /* Blank or comment */ },
                _ => {
                    let msg =
                        format!("Error parsing Secretfile line: {}", &line);
                    return Err(err(msg));
                }
            }
        }
        Ok(sf)
    }

    /// Read a Secretfile from a string.  Currently only used for testing.
    #[cfg(test)]
    pub fn from_str<S: AsRef<str>>(s: S) -> Result<Secretfile, BoxedError> {
        let mut cursor = io::Cursor::new(s.as_ref().as_bytes());
        Secretfile::read(&mut cursor)
    }

    /// The default Secretfile.
    pub fn default() -> Result<Secretfile, BoxedError> {
        let mut path = try!(env::current_dir());
        path.push("Secretfile");
        Secretfile::read(&mut try!(File::open(path)))
    }


    pub fn get(&self, name: &str) -> Option<&Location> {
        self.mappings.get(name)
    }
}

#[test]
fn test_parse() {
    let data = "\
# This is a comment.

FOO_USERNAME secret/$SECRET_NAME:username\n\
FOO_PASSWORD secret/${SECRET_NAME}:password\n\
";
    env::set_var("SECRET_NAME", "foo");
    let secretfile = Secretfile::from_str(data).unwrap();
    assert_eq!(&Location::Keyed("secret/foo".to_owned(), "username".to_owned()),
               secretfile.get("FOO_USERNAME").unwrap());
    assert_eq!(&Location::Keyed("secret/foo".to_owned(), "password".to_owned()),
               secretfile.get("FOO_PASSWORD").unwrap());
}
