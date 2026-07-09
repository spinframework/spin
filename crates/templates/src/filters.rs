// liquid_derive::FilterReflection violates this lint
#![allow(clippy::box_default)]

use heck::{ToKebabCase, ToSnakeCase, ToUpperCamelCase};
use itertools::Itertools;
use liquid_core::{Filter, ParseFilter, Runtime, ValueView};
use liquid_derive::FilterReflection;

// Filters that are added to the Liquid parser and allow templates to specify
// transformations of input strings

// ADDING A FILTER HERE IS NOT ENOUGH.  You must also:
//
// * Register it with the template ParserBuilder in `Run::template_parser`.
//   Otherwise Liquid won't know about it.
// * Add a test using the registration name to `test_filters` in run.rs.
//   This ensures that the name->parser->filter chain is hooked up correctly.

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "kebab_case",
    description = "Change text to kebab-case.",
    parsed(KebabCaseFilter)
)]
pub(crate) struct KebabCaseFilterParser;

#[derive(Debug, Default, liquid_derive::Display_filter)]
#[name = "kebab_case"]
struct KebabCaseFilter;

impl Filter for KebabCaseFilter {
    fn evaluate(
        &self,
        input: &dyn ValueView,
        _runtime: &dyn Runtime,
    ) -> Result<liquid_core::model::Value, liquid_core::error::Error> {
        let input = input
            .as_scalar()
            .ok_or_else(|| liquid_core::error::Error::with_msg("String expected"))?;

        let input = input.into_string().to_string().to_kebab_case();
        Ok(liquid_core::model::Value::scalar(input))
    }
}

#[derive(Clone, liquid_derive::ParseFilter, liquid_derive::FilterReflection)]
#[filter(
    name = "pascal_case",
    description = "Change text to PascalCase.",
    parsed(PascalCaseFilter)
)]
pub(crate) struct PascalCaseFilterParser;

#[derive(Debug, Default, liquid_derive::Display_filter)]
#[name = "pascal_case"]
struct PascalCaseFilter;

impl Filter for PascalCaseFilter {
    fn evaluate(
        &self,
        input: &dyn ValueView,
        _runtime: &dyn Runtime,
    ) -> Result<liquid::model::Value, liquid_core::error::Error> {
        let input = input
            .as_scalar()
            .ok_or_else(|| liquid_core::error::Error::with_msg("String expected"))?;

        let input = input.into_string().to_string().to_upper_camel_case();
        Ok(liquid::model::Value::scalar(input))
    }
}

#[derive(Clone, liquid_derive::ParseFilter, liquid_derive::FilterReflection)]
#[filter(
    name = "snake_case",
    description = "Change text to snake_case.",
    parsed(SnakeCaseFilter)
)]
pub(crate) struct SnakeCaseFilterParser;

#[derive(Debug, Default, liquid_derive::Display_filter)]
#[name = "snake_case"]
struct SnakeCaseFilter;

impl Filter for SnakeCaseFilter {
    fn evaluate(
        &self,
        input: &dyn ValueView,
        _runtime: &dyn Runtime,
    ) -> Result<liquid::model::Value, liquid_core::error::Error> {
        let input = input
            .as_scalar()
            .ok_or_else(|| liquid_core::error::Error::with_msg("String expected"))?;

        let input = input.into_string().to_string().to_snake_case();
        Ok(input.to_value())
    }
}

#[derive(Clone, liquid_derive::ParseFilter, liquid_derive::FilterReflection)]
#[filter(
    name = "http_wildcard",
    description = "Add Spin HTTP wildcard suffix (/...) if needed.",
    parsed(HttpWildcardFilter)
)]
pub(crate) struct HttpWildcardFilterParser;

#[derive(Debug, Default, liquid_derive::Display_filter)]
#[name = "http_wildcard"]
struct HttpWildcardFilter;

impl Filter for HttpWildcardFilter {
    fn evaluate(
        &self,
        input: &dyn ValueView,
        _runtime: &dyn Runtime,
    ) -> Result<liquid::model::Value, liquid_core::error::Error> {
        let input = input
            .as_scalar()
            .ok_or_else(|| liquid_core::error::Error::with_msg("String expected"))?;

        let route = input.into_string().to_string();
        let wildcard_route = if route.ends_with("/...") {
            route
        } else if route.ends_with('/') {
            format!("{route}...")
        } else {
            format!("{route}/...")
        };

        Ok(wildcard_route.to_value())
    }
}

#[derive(Clone, ParseFilter, FilterReflection)]
#[filter(
    name = "dotted_pascal_case",
    description = "Change text to Dotted.Pascal.Case.",
    parsed(DottedPascalCaseFilter)
)]
pub(crate) struct DottedPascalCaseFilterParser;

#[derive(Debug, Default, liquid_derive::Display_filter)]
#[name = "dotted_pascal_case"]
struct DottedPascalCaseFilter;

impl Filter for DottedPascalCaseFilter {
    fn evaluate(
        &self,
        input: &dyn ValueView,
        _runtime: &dyn Runtime,
    ) -> Result<liquid_core::model::Value, liquid_core::error::Error> {
        let input = input
            .as_scalar()
            .ok_or_else(|| liquid_core::error::Error::with_msg("String expected"))?;

        let input = input.into_string().to_string();

        let result = input.split('.').map(|s| s.to_upper_camel_case()).join(".");

        Ok(result.to_value())
    }
}

#[cfg(test)]
mod test {
    use liquid_core::{Filter, ValueView};

    // Just to save cluttering the tests with ceremonial bits
    fn dotted_pascal(input: &str) -> String {
        let filter = super::DottedPascalCaseFilter;
        let runtime = liquid_core::runtime::RuntimeBuilder::new().build();
        filter
            .evaluate(&input.to_value(), &runtime)
            .map(|v| v.into_scalar().unwrap().into_string().to_string())
            .unwrap()
    }

    #[test]
    fn test_dotted_pascal_case() {
        assert_eq!("Fermyon.PetStore", dotted_pascal("Fermyon.PetStore"));
        assert_eq!("FermyonPetStore", dotted_pascal("fermyon-pet-store"));
        assert_eq!("FermyonPetStore", dotted_pascal("fermyon_pet_store"));
        assert_eq!("Fermyon.PetStore", dotted_pascal("fermyon.pet-store"));
        assert_eq!("Fermyon.PetStore", dotted_pascal("fermyon.pet_store"));
        assert_eq!("FermyonPetStore", dotted_pascal("fermyon pet store"));
    }
}
