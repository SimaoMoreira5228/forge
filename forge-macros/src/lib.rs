use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use syn::{
	Attribute, Expr, ExprLit, FnArg, ImplItem, ItemImpl, Lit, LitStr, Meta, MetaNameValue, ReturnType, Type,
	parse_macro_input,
};

#[proc_macro_attribute]
pub fn lua_api(args: TokenStream, input: TokenStream) -> TokenStream {
	let args: syn::punctuated::Punctuated<Meta, syn::Token![,]> =
		parse_macro_input!(args with syn::punctuated::Punctuated::parse_terminated);
	let input = parse_macro_input!(input as ItemImpl);

	let api_name = extract_api_name_from_args(&args);

	let type_name = if let Type::Path(type_path) = &*input.self_ty {
		type_path.path.segments.last().unwrap().ident.clone()
	} else {
		panic!("lua_api macro only supports named types");
	};

	let lua_functions = extract_lua_functions(&input);

	let create_table_fn = generate_create_table_function(&api_name, &lua_functions, &type_name);

	let type_definitions_fn = generate_type_definitions_function(&api_name, &lua_functions, &type_name);

	let original_impl = &input;

	let output = quote! {
		#original_impl

		#create_table_fn

		#type_definitions_fn
	};

	output.into()
}

fn extract_api_name_from_args(args: &syn::punctuated::Punctuated<Meta, syn::Token![,]>) -> String {
	for arg in args {
		if let Meta::NameValue(MetaNameValue { path, value, .. }) = arg {
			if path.is_ident("name") {
				if let Expr::Lit(ExprLit { lit: Lit::Str(s), .. }) = value {
					return s.value();
				}
			}
		}
	}
	panic!("lua_api macro requires name argument: #[lua_api(name = \"api_name\")]");
}

struct LuaFunction {
	name: String,
	doc_comment: Option<String>,
	args: Vec<(String, String)>,
	return_type: Option<String>,
	fn_ident: Ident,
	has_self: bool,
	has_lua_context: bool,
}

fn extract_lua_functions(input: &ItemImpl) -> Vec<LuaFunction> {
	let mut functions = Vec::new();

	for item in &input.items {
		if let ImplItem::Fn(method) = item {
			let name = method.sig.ident.to_string();
			let fn_ident = method.sig.ident.clone();

			let doc_comment = extract_doc_comment(&method.attrs);

			let has_self = method.sig.inputs.iter().any(|arg| matches!(arg, FnArg::Receiver(_)));

			let has_lua_context = method.sig.inputs.iter().any(|arg| {
				if let FnArg::Typed(pat_type) = arg {
					if let Type::Reference(type_ref) = &*pat_type.ty {
						if let Type::Path(type_path) = &*type_ref.elem {
							if let Some(segment) = type_path.path.segments.last() {
								return segment.ident == "Lua";
							}
						}
					}
				}
				false
			});

			let args = extract_function_args(&method.sig.inputs, has_self, has_lua_context);

			let return_type = extract_return_type(&method.sig.output);

			functions.push(LuaFunction {
				name,
				doc_comment,
				args,
				return_type,
				fn_ident,
				has_self,
				has_lua_context,
			});
		}
	}

	functions
}

fn extract_doc_comment(attrs: &[Attribute]) -> Option<String> {
	let mut comment = String::new();

	for attr in attrs {
		if attr.path().is_ident("doc") {
			if let Meta::NameValue(name_value) = &attr.meta {
				if let Expr::Lit(ExprLit {
					lit: Lit::Str(lit_str), ..
				}) = &name_value.value
				{
					let line = lit_str.value().trim().to_string();
					if !comment.is_empty() {
						comment.push(' ');
					}
					comment.push_str(&line);
				}
			}
		}
	}

	if comment.is_empty() { None } else { Some(comment) }
}

fn extract_function_args(
	inputs: &syn::punctuated::Punctuated<FnArg, syn::Token![,]>,
	exclude_self: bool,
	exclude_lua_context: bool,
) -> Vec<(String, String)> {
	let mut args = Vec::new();

	for input in inputs {
		match input {
			FnArg::Receiver(_) if exclude_self => {
				continue;
			}
			FnArg::Typed(pat_type) => {
				if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
					let name = pat_ident.ident.to_string();

					if exclude_lua_context {
						if let Type::Reference(type_ref) = &*pat_type.ty {
							if let Type::Path(type_path) = &*type_ref.elem {
								if let Some(segment) = type_path.path.segments.last() {
									if segment.ident == "Lua" {
										continue;
									}
								}
							}
						}
					}

					let type_str = type_to_lua_type(&pat_type.ty);
					args.push((name, type_str));
				}
			}
			FnArg::Receiver(_) => {
				args.push(("self".to_string(), "table".to_string()));
			}
		}
	}

	args
}

fn extract_return_type(output: &ReturnType) -> Option<String> {
	match output {
		ReturnType::Default => None,
		ReturnType::Type(_, ty) => Some(type_to_lua_type(ty)),
	}
}

fn type_to_lua_type(ty: &Type) -> String {
	match ty {
		Type::Path(path) => {
			let segment = path.path.segments.last().unwrap();
			match segment.ident.to_string().as_str() {
				"String" => "string".to_string(),
				"bool" => "boolean".to_string(),
				"i32" | "i64" | "u32" | "u64" | "f32" | "f64" | "usize" | "isize" => "number".to_string(),
				"Vec" => {
					if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
						if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
							let inner_type = type_to_lua_type(inner_ty);
							return format!("{}[]", inner_type);
						}
					}
					"table".to_string()
				}
				"Option" => {
					if let syn::PathArguments::AngleBracketed(args) = &segment.arguments {
						if let Some(syn::GenericArgument::Type(inner_ty)) = args.args.first() {
							let inner_type = type_to_lua_type(inner_ty);
							return format!("{}?", inner_type);
						}
					}
					"any?".to_string()
				}
				_ => "any".to_string(),
			}
		}
		_ => "any".to_string(),
	}
}

fn generate_create_table_function(api_name: &str, functions: &[LuaFunction], type_name: &Ident) -> proc_macro2::TokenStream {
	let static_methods: Vec<_> = functions.iter().filter(|f| !f.has_self).collect();
	let instance_methods: Vec<_> = functions.iter().filter(|f| f.has_self).collect();

	let static_bindings = static_methods
		.iter()
		.map(|func| {
			let func_name = &func.name;
			let func_ident = &func.fn_ident;

			if func.has_lua_context {
				let param_pattern = generate_param_pattern(&func.args);
				let call_args = generate_call_args(&func.args);

				quote! {
					let #func_ident = lua.create_function(|lua, #param_pattern| {
						let result = #type_name::#func_ident(lua, #call_args);
						Ok(result)
					})?;
					tbl.set(#func_name, #func_ident)?;
				}
			} else {
				let param_pattern = generate_param_pattern(&func.args);
				let call_args = generate_call_args(&func.args);

				quote! {
					let #func_ident = lua.create_function(|_, #param_pattern| {
						let result = #type_name::#func_ident(#call_args);
						Ok(result)
					})?;
					tbl.set(#func_name, #func_ident)?;
				}
			}
		})
		.collect::<Vec<_>>();

	let create_fn_name = Ident::new(&format!("create_{}_table", api_name), Span::call_site());

	if instance_methods.is_empty() {
		quote! {
			impl #type_name {
				pub fn #create_fn_name(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
					let tbl = lua.create_table()?;

					#(#static_bindings)*

					Ok(tbl)
				}
			}
		}
	} else {
		let instance_bindings = instance_methods
			.iter()
			.map(|func| {
				let func_name = &func.name;
				let func_ident = &func.fn_ident;

				let args_without_self: Vec<_> = func.args.iter().filter(|(name, _)| name != "self").collect();
				let param_pattern = if args_without_self.is_empty() {
					quote! { _: () }
				} else if args_without_self.len() == 1 {
					let (name, _) = &args_without_self[0];
					let param_ident = Ident::new(name, Span::call_site());
					quote! { #param_ident }
				} else {
					let params: Vec<_> = args_without_self
						.iter()
						.map(|(name, _)| Ident::new(name, Span::call_site()))
						.collect();
					quote! { (#(#params),*) }
				};

				let method_args = if func.has_lua_context {
					if args_without_self.is_empty() {
						quote! { lua }
					} else if args_without_self.len() == 1 {
						let (name, _) = &args_without_self[0];
						let arg_ident = Ident::new(name, Span::call_site());
						quote! { lua, #arg_ident }
					} else {
						let args: Vec<_> = args_without_self
							.iter()
							.map(|(name, _)| Ident::new(name, Span::call_site()))
							.collect();
						quote! { lua, #(#args),* }
					}
				} else {
					if args_without_self.is_empty() {
						quote! {}
					} else if args_without_self.len() == 1 {
						let (name, _) = &args_without_self[0];
						let arg_ident = Ident::new(name, Span::call_site());
						quote! { #arg_ident }
					} else {
						let args: Vec<_> = args_without_self
							.iter()
							.map(|(name, _)| Ident::new(name, Span::call_site()))
							.collect();
						quote! { #(#args),* }
					}
				};

				if func.has_lua_context {
					quote! {
						let #func_ident = {
							let instance = self.clone();
							lua.create_function(move |lua, #param_pattern| {
								let result = instance.#func_ident(#method_args);
								Ok(result)
							})?
						};
						tbl.set(#func_name, #func_ident)?;
					}
				} else {
					quote! {
						let #func_ident = {
							let instance = self.clone();
							lua.create_function(move |_, #param_pattern| {
								let result = instance.#func_ident(#method_args);
								Ok(result)
							})?
						};
						tbl.set(#func_name, #func_ident)?;
					}
				}
			})
			.collect::<Vec<_>>();
		let static_bindings_for_static_table = static_methods
			.iter()
			.map(|func| {
				let func_name = &func.name;
				let func_ident = &func.fn_ident;

				if func.has_lua_context {
					let param_pattern = generate_param_pattern(&func.args);
					let call_args = generate_call_args(&func.args);

					quote! {
						let #func_ident = lua.create_function(|lua, #param_pattern| {
							let result = #type_name::#func_ident(lua, #call_args);
							Ok(result)
						})?;
						tbl.set(#func_name, #func_ident)?;
					}
				} else {
					let param_pattern = generate_param_pattern(&func.args);
					let call_args = generate_call_args(&func.args);

					quote! {
						let #func_ident = lua.create_function(|_, #param_pattern| {
							let result = #type_name::#func_ident(#call_args);
							Ok(result)
						})?;
						tbl.set(#func_name, #func_ident)?;
					}
				}
			})
			.collect::<Vec<_>>();

		quote! {
			impl #type_name {
				pub fn #create_fn_name(&self, lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
					let tbl = lua.create_table()?;

					#(#static_bindings)*
					#(#instance_bindings)*

					Ok(tbl)
				}

				pub fn create_static_table(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
					let tbl = lua.create_table()?;

					#(#static_bindings_for_static_table)*

					Ok(tbl)
				}
			}
		}
	}
}

fn generate_param_pattern(args: &[(String, String)]) -> proc_macro2::TokenStream {
	if args.is_empty() {
		quote! { _: () }
	} else if args.len() == 1 && args[0].0 != "self" {
		let param_name = Ident::new(&args[0].0, Span::call_site());
		quote! { #param_name }
	} else {
		let params: Vec<_> = args
			.iter()
			.filter(|(name, _)| name != "self")
			.map(|(name, _)| Ident::new(name, Span::call_site()))
			.collect();
		if params.is_empty() {
			quote! { _: () }
		} else {
			quote! { (#(#params),*) }
		}
	}
}

fn generate_call_args(args: &[(String, String)]) -> proc_macro2::TokenStream {
	let filtered_args: Vec<_> = args.iter().filter(|(name, _)| name != "self").collect();

	if filtered_args.is_empty() {
		quote! {}
	} else if filtered_args.len() == 1 {
		let arg_name = Ident::new(&filtered_args[0].0, Span::call_site());
		quote! { #arg_name }
	} else {
		let args: Vec<_> = filtered_args
			.iter()
			.map(|(name, _)| Ident::new(name, Span::call_site()))
			.collect();
		quote! { #(#args),* }
	}
}

fn generate_type_definitions_function(
	api_name: &str,
	functions: &[LuaFunction],
	type_name: &Ident,
) -> proc_macro2::TokenStream {
	let class_name = capitalize_first_letter(api_name);
	let mut type_def = format!("---@class {}\n", class_name);

	for func in functions {
		let display_args: Vec<_> = func.args.iter().filter(|(name, _)| name != "self").collect();

		let args_str = display_args
			.iter()
			.map(|(name, typ)| format!("{}: {}", name, typ))
			.collect::<Vec<_>>()
			.join(", ");

		let return_str = func.return_type.as_deref().unwrap_or("nil");

		if let Some(comment) = &func.doc_comment {
			type_def.push_str(&format!("--- {}\n", comment));
		}

		if func.has_self {
			type_def.push_str(&format!("---@field {} fun({}): {}\n", func.name, args_str, return_str));
		} else {
			type_def.push_str(&format!("---@field {} fun({}): {}\n", func.name, args_str, return_str));
		}
	}

	type_def.push('\n');
	type_def.push_str(&format!("---@type {}\n", class_name));

	let type_def_lit = LitStr::new(&type_def, Span::call_site());
	let type_def_fn_name = Ident::new(&format!("{}_lua_type_definitions", api_name), Span::call_site());

	quote! {
		impl #type_name {
			pub fn #type_def_fn_name() -> &'static str {
				#type_def_lit
			}
		}
	}
}

fn capitalize_first_letter(s: &str) -> String {
	let mut chars = s.chars();
	match chars.next() {
		None => String::new(),
		Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use syn::parse_quote;

	#[test]
	fn test_extract_api_name_from_args() {
		let args: syn::punctuated::Punctuated<Meta, syn::Token![,]> = parse_quote!(name = "test_api");
		let name = extract_api_name_from_args(&args);
		assert_eq!(name, "test_api");
	}

	#[test]
	fn test_type_to_lua_type() {
		let ty: Type = parse_quote!(String);
		assert_eq!(type_to_lua_type(&ty), "string");

		let ty: Type = parse_quote!(bool);
		assert_eq!(type_to_lua_type(&ty), "boolean");

		let ty: Type = parse_quote!(i32);
		assert_eq!(type_to_lua_type(&ty), "number");

		let ty: Type = parse_quote!(Vec<String>);
		assert_eq!(type_to_lua_type(&ty), "string[]");

		let ty: Type = parse_quote!(Option<String>);
		assert_eq!(type_to_lua_type(&ty), "string?");
	}

	#[test]
	fn test_capitalize_first_letter() {
		assert_eq!(capitalize_first_letter("hello"), "Hello");
		assert_eq!(capitalize_first_letter(""), "");
		assert_eq!(capitalize_first_letter("a"), "A");
		assert_eq!(capitalize_first_letter("API"), "API");
	}
}
