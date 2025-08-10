use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOr<T: From<String>> {
    String(String),
    Value(T),
}

impl<T: From<String>> StringOr<T> {
    pub fn into_value(self) -> T {
        match self {
            StringOr::String(s) => T::from(s),
            StringOr::Value(v) => v,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T: From<String>> From<String> for OneOrMany<T> {
    fn from(value: String) -> Self {
        OneOrMany::One(T::from(value))
    }
}
