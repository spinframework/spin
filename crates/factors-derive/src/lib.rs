use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Data, DeriveInput, Error};

#[proc_macro_derive(RuntimeFactors)]
pub fn derive_factors(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let expanded = expand_factors(&input).unwrap_or_else(|err| err.into_compile_error());

    #[cfg(feature = "expander")]
    let expanded = if let Some(dest_dir) = std::env::var_os("SPIN_FACTORS_DERIVE_EXPAND_DIR") {
        expander::Expander::new("factors")
            .write_to(expanded, std::path::Path::new(&dest_dir))
            .unwrap()
    } else {
        expanded
    };

    expanded.into()
}

#[allow(non_snake_case)]
fn expand_factors(input: &DeriveInput) -> syn::Result<TokenStream> {
    let name = &input.ident;
    let vis = &input.vis;

    let app_state_name = format_ident!("{name}AppState");
    let builders_name = format_ident!("{name}InstanceBuilders");
    let state_name = format_ident!("{name}InstanceState");
    let runtime_config_name = format_ident!("{name}RuntimeConfig");

    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            input,
            "cannot derive Factors for generic structs",
        ));
    }

    // Get struct fields
    let fields = match &input.data {
        Data::Struct(struct_data) => &struct_data.fields,
        _ => {
            return Err(Error::new_spanned(
                input,
                "can only derive Factors for structs",
            ))
        }
    };
    let mut factor_names = Vec::with_capacity(fields.len());
    let mut factor_types = Vec::with_capacity(fields.len());
    for field in fields.iter() {
        factor_names.push(
            field
                .ident
                .as_ref()
                .ok_or_else(|| Error::new_spanned(input, "tuple structs are not supported"))?,
        );
        factor_types.push(&field.ty);
    }

    let Any = quote!(::std::any::Any);
    let Send = quote!(::std::marker::Send);
    let TypeId = quote!(::std::any::TypeId);
    let factors_crate = format_ident!("spin_factors");
    let factors_path = quote!(::#factors_crate);
    let wasmtime = quote!(#factors_path::wasmtime);
    let ResourceTable = quote!(#wasmtime::component::ResourceTable);
    let Result = quote!(#factors_path::Result);
    let Error = quote!(#factors_path::Error);
    let Factor = quote!(#factors_path::Factor);
    let ConfiguredApp = quote!(#factors_path::ConfiguredApp);
    let FactorInstanceBuilder = quote!(#factors_path::FactorInstanceBuilder);

    Ok(quote! {
        impl #factors_path::RuntimeFactors for #name {
            type AppState = #app_state_name;
            type InstanceBuilders = #builders_name;
            type InstanceState = #state_name;
            type RuntimeConfig = #runtime_config_name;

            fn init<T: #factors_path::AsInstanceState<Self::InstanceState> + Send + 'static>(
                &mut self,
                linker: &mut #wasmtime::component::Linker<T>,
            ) -> #Result<()> {
                let factor_type_ids = [#(
                    (stringify!(#factor_types), #TypeId::of::<(<#factor_types as #Factor>::InstanceBuilder, <#factor_types as #Factor>::AppState)>()),
                )*];

                let mut unique = ::std::collections::HashSet::new();
                for (name, type_id) in factor_type_ids {
                    if !unique.insert(type_id) {
                        return Err(#Error::DuplicateFactorTypes(name.to_owned()));
                    }
                }

                #(
                    #[allow(non_camel_case_types)]
                    struct #factor_names;

                    impl #factors_path::FactorField for #factor_names {
                        type State = #state_name;
                        type Factor = #factor_types;

                        fn get(state: &mut #state_name) -> (
                            &mut #factors_path::FactorInstanceState<#factor_types>,
                            &mut #wasmtime::component::ResourceTable,
                        ) {
                            (&mut state.#factor_names, &mut state.__table)
                        }
                    }

                    #Factor::init(
                        &mut self.#factor_names,
                        &mut #factors_path::FactorInitContext::<'_, T, #factor_names> {
                            linker,
                            _marker: std::marker::PhantomData,
                        },
                    ).map_err(#Error::factor_init_error::<#factor_types>)?;
                )*
                Ok(())
            }

            fn configure_app(
                &self,
                app: #factors_path::App,
                runtime_config: Self::RuntimeConfig,
            ) -> #Result<#ConfiguredApp<Self>> {
                let mut app_state = #app_state_name {
                    #( #factor_names: None, )*
                };
                #(
                    app_state.#factor_names = Some(
                        #Factor::configure_app(
                            &self.#factor_names,
                            #factors_path::ConfigureAppContext::<Self, #factor_types>::new(
                                &app,
                                &app_state,
                                runtime_config.#factor_names,
                            )?,
                        ).map_err(#Error::factor_configure_app_error::<#factor_types>)?
                    );
                )*
                Ok(#ConfiguredApp::new(app, app_state))
            }

            fn prepare(
                &self, configured_app: &#ConfiguredApp<Self>,
                component_id: &str,
            ) -> #Result<Self::InstanceBuilders> {
                let app_component = configured_app.app().get_component(component_id).ok_or_else(|| {
                    #factors_path::Error::UnknownComponent(component_id.to_string())
                })?;
                let mut builders = #builders_name {
                    #( #factor_names: None, )*
                };
                #(
                    builders.#factor_names = Some(
                        #Factor::prepare::<Self>(
                            &self.#factor_names,
                            #factors_path::PrepareContext::new(
                                configured_app.app_state::<#factor_types>().unwrap(),
                                &app_component,
                                &mut builders,
                            ),
                        ).map_err(#Error::factor_prepare_error::<#factor_types>)?
                    );
                )*
                Ok(builders)
            }

            fn build_instance_state(
                &self,
                builders: Self::InstanceBuilders,
            ) -> #Result<Self::InstanceState> {
                Ok(#state_name {
                    __table: #ResourceTable::new(),
                    #(
                        #factor_names: #FactorInstanceBuilder::build(
                            builders.#factor_names.unwrap()
                        ).map_err(#Error::factor_build_error::<#factor_types>)?,
                    )*
                })
            }

            fn app_state<F: #Factor>(app_state: &Self::AppState) -> Option<&F::AppState> {
                #(
                    if let Some(state) = &app_state.#factor_names {
                        if let Some(state) = <dyn #Any>::downcast_ref(state) {
                            return Some(state)
                        }
                    }
                )*
                None
            }

            fn instance_builder_mut<F: #Factor>(
                builders: &mut Self::InstanceBuilders,
            ) -> Option<Option<&mut F::InstanceBuilder>> {
                let type_id = #TypeId::of::<(F::InstanceBuilder, F::AppState)>();
                #(
                    if type_id == #TypeId::of::<(<#factor_types as #Factor>::InstanceBuilder, <#factor_types as #Factor>::AppState)>() {
                        return Some(
                            builders.#factor_names.as_mut().map(|builder| {
                                <dyn #Any>::downcast_mut(builder).unwrap()
                            })
                        );
                    }
                )*
                None
            }
        }

        #vis struct #app_state_name {
            #(
                pub #factor_names: Option<<#factor_types as #Factor>::AppState>,
            )*
        }

        #vis struct #builders_name {
            #(
                #factor_names: Option<<#factor_types as #Factor>::InstanceBuilder>,
            )*
        }

        #[allow(dead_code)]
        impl #builders_name {
            #(
                pub fn #factor_names(&mut self) -> &mut <#factor_types as #Factor>::InstanceBuilder {
                    self.#factor_names.as_mut().unwrap()
                }
            )*
        }

        impl #factors_path::HasInstanceBuilder for #builders_name {
            fn for_factor<F: #Factor>(
                &mut self
            ) -> Option<&mut F::InstanceBuilder> {
                let type_id = #TypeId::of::<F::InstanceBuilder>();
                #(
                    if type_id == #TypeId::of::<<#factor_types as #Factor>::InstanceBuilder>() {
                        let builder = self.#factor_names.as_mut().unwrap();
                        return Some(
                            <dyn #Any>::downcast_mut(builder).unwrap()
                        );
                    }
                )*
                None
            }
        }

        #vis struct #state_name {
            __table: #ResourceTable,
            #(
                pub #factor_names: #factors_path::FactorInstanceState<#factor_types>,
            )*
        }

        impl #factors_path::RuntimeFactorsInstanceState for #state_name {
            fn get_with_table<F: #Factor>(
                &mut self
            ) -> ::std::option::Option<(&mut #factors_path::FactorInstanceState<F>, &mut #ResourceTable)> {
                #(
                    if let Some(state) = (&mut self.#factor_names as &mut (dyn #Any + #Send)).downcast_mut() {
                        return Some((state, &mut self.__table))
                    }
                )*
                None
            }

            fn table(&self) -> &#ResourceTable {
                &self.__table
            }

            fn table_mut(&mut self) -> &mut #ResourceTable {
                &mut self.__table
            }
        }

        impl #factors_path::AsInstanceState<#state_name> for #state_name {
            fn as_instance_state(&mut self) -> &mut Self {
                self
            }
        }

        #[derive(Default)]
        #vis struct #runtime_config_name {
            #(
                pub #factor_names: Option<<#factor_types as #Factor>::RuntimeConfig>,
            )*
        }

        impl #runtime_config_name {
            /// Get the runtime configuration from the given source.
            #[allow(dead_code)]
            pub fn from_source<T>(mut source: T) -> anyhow::Result<Self>
                where T: #(#factors_path::FactorRuntimeConfigSource<#factor_types> +)* #factors_path::RuntimeConfigSourceFinalizer
            {
                #(
                    let #factor_names = <T as #factors_path::FactorRuntimeConfigSource<#factor_types>>::get_runtime_config(&mut source)?;
                )*
                source.finalize()?;
                Ok(#runtime_config_name {
                    #(
                        #factor_names,
                    )*
                })
            }
        }
    })
}
