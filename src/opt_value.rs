pub(crate) const FLAG_NOT_PRESENT: &str = "<< flag not present >>";
pub(crate) const FLAG_PRESENT_BUT_NO_VALUE: &str = "<< flag present but no value >>";

pub(crate) enum OptionalValueFlag {
    NotPresent,
    PresentButNoValue,
    Present(String),
}

impl From<&String> for OptionalValueFlag {
    fn from(value: &String) -> Self {
        match value.as_str() {
            FLAG_NOT_PRESENT => Self::NotPresent,
            FLAG_PRESENT_BUT_NO_VALUE => Self::PresentButNoValue,
            _ => Self::Present(value.clone()),
        }
    }
}

impl OptionalValueFlag {
    pub fn is_present(&self) -> bool {
        !matches!(self, Self::NotPresent)
    }
}
