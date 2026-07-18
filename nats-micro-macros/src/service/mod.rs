use proc_macro2::TokenStream;
use quote::quote;
use syn::{ImplItem, ImplItemFn, ItemImpl, Type};

mod args;
mod classify;
mod client;
mod consumer;
mod contract;
mod dispatch;
mod local;
mod metadata;
mod parse;
mod startup;
mod validate;

pub(crate) use args::{AuthIntent, ServiceArgs};
pub(crate) use classify::{
    ArgumentKind, ArgumentModel, PayloadModel, ResponseModel, WireCodec, Wrapper,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Request,
    Publish,
    Subscribe,
    Consumer,
}

#[derive(Debug, Clone)]
pub(crate) struct SubjectModel {
    pub template: String,
    pub pattern: String,
    pub placeholders: Vec<(String, usize)>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OperationOptions {
    pub subject: Option<String>,
    pub queue: Option<String>,
    pub concurrency: Option<usize>,
    pub timeout_ms: Option<u64>,
    pub auth: Option<AuthIntent>,
    pub response: Option<Type>,
    pub stream: Option<String>,
    pub durable: Option<String>,
    pub filter: Option<String>,
    pub ack_wait_ms: Option<u64>,
    pub max_deliver: Option<i64>,
    pub backoff_ms: Vec<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct OperationModel {
    pub kind: OperationKind,
    pub method: ImplItemFn,
    pub options: OperationOptions,
    pub subject: SubjectModel,
    pub arguments: Vec<ArgumentModel>,
    pub response: ResponseModel,
    pub operation_index: Option<usize>,
    pub consumer_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub(crate) enum MethodModel {
    Request(OperationModel),
    Publish(OperationModel),
    Subscribe(OperationModel),
    Consumer(OperationModel),
    Ordinary(ImplItemFn),
}

impl MethodModel {
    pub(crate) fn operation(&self) -> Option<&OperationModel> {
        match self {
            Self::Request(model)
            | Self::Publish(model)
            | Self::Subscribe(model)
            | Self::Consumer(model) => Some(model),
            Self::Ordinary(_) => None,
        }
    }
}

#[allow(dead_code)]
pub(crate) type RequestModel = OperationModel;
#[allow(dead_code)]
pub(crate) type PublishModel = OperationModel;
#[allow(dead_code)]
pub(crate) type SubscribeModel = OperationModel;
#[allow(dead_code)]
pub(crate) type ConsumerModel = OperationModel;

#[derive(Debug, Clone)]
pub(crate) struct ServiceModel {
    pub args: ServiceArgs,
    pub service_ident: syn::Ident,
    pub state_type: Type,
    pub item_impl: ItemImpl,
    pub methods: Vec<MethodModel>,
}

pub(crate) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    match expand_result(args, input) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error(),
    }
}

fn expand_result(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let args = ServiceArgs::parse(args)?;
    let mut model = parse::build_model(args, input)?;
    validate::validate_service(&model)?;
    parse::assign_indexes(&mut model);

    let service_ident = &model.service_ident;
    let cleaned_impl = cleaned_impl(&model);
    let metadata = metadata::generate(&model);
    let dispatch = dispatch::generate(&model);
    let consumers = consumer::generate(&model);
    let client = client::generate(&model);
    let local = local::generate(&model);
    let contract = contract::generate(&model);
    let startup = startup::generate(&model);

    Ok(quote! {
        #[derive(Debug, Clone, Copy, Default)]
        pub struct #service_ident;

        #cleaned_impl
        #metadata
        #dispatch
        #consumers
        #client
        #local
        #contract
        #startup
    })
}

fn cleaned_impl(model: &ServiceModel) -> ItemImpl {
    let mut item_impl = model.item_impl.clone();
    let mut methods = model.methods.iter();
    item_impl.items = item_impl
        .items
        .iter()
        .map(|item| match item {
            ImplItem::Fn(_) => {
                let method = methods.next().expect("method model");
                let (mut method, operation) = match method {
                    MethodModel::Request(model)
                    | MethodModel::Publish(model)
                    | MethodModel::Subscribe(model)
                    | MethodModel::Consumer(model) => (model.method.clone(), true),
                    MethodModel::Ordinary(method) => (method.clone(), false),
                };
                if operation {
                    method
                        .attrs
                        .push(syn::parse_quote!(#[allow(dead_code, clippy::unused_async)]));
                }
                ImplItem::Fn(method)
            }
            other => other.clone(),
        })
        .collect();
    item_impl
}

#[cfg(test)]
mod tests;
