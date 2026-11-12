//! Language-neutral facts. Unknown facts stay unknown; names are never type evidence.
use serde::{Deserialize, Serialize};

macro_rules! vocabulary {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { #[default] Unknown, $($variant),* }
    };
}
vocabulary!(SymbolKind {
    Function,
    Method,
    Constructor,
    Destructor,
    Concept,
    Field,
    Property,
    Variable,
    Constant,
    Parameter,
    Class,
    Struct,
    Union,
    Interface,
    Trait,
    Enum,
    EnumMember,
    TypeAlias,
    Type,
    Module,
    Namespace,
    Impl,
    Macro,
    Component,
    Object,
    Record
});
vocabulary!(SymbolRole {
    Declaration,
    Definition,
    Accessor,
    AssociatedFunction
});
vocabulary!(Visibility {
    Public,
    Private,
    Protected,
    Internal,
    Package,
    Restricted
});
vocabulary!(ScopeKind {
    Module,
    Namespace,
    Type,
    Function,
    Block
});
vocabulary!(RelationKind {
    Inherits,
    Implements,
    Overrides,
    Embeds
});

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl std::ops::Deref for SymbolKind {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl PartialEq<str> for SymbolKind {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for SymbolKind {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
impl From<&str> for SymbolKind {
    fn from(kind: &str) -> Self {
        Self::from_legacy(kind)
    }
}
impl From<String> for SymbolKind {
    fn from(kind: String) -> Self {
        Self::from_legacy(&kind)
    }
}
impl SymbolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Function => "function",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Destructor => "destructor",
            Self::Concept => "concept",
            Self::Field => "field",
            Self::Property => "property",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::Parameter => "parameter",
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Union => "union",
            Self::Interface => "interface",
            Self::Trait => "trait",
            Self::Enum => "enum",
            Self::EnumMember => "enum_member",
            Self::TypeAlias => "type_alias",
            Self::Type => "type",
            Self::Module => "module",
            Self::Namespace => "namespace",
            Self::Impl => "impl",
            Self::Macro => "macro",
            Self::Component => "component",
            Self::Object => "object",
            Self::Record => "record",
        }
    }

    pub fn from_legacy(kind: &str) -> Self {
        match kind {
            "function" => Self::Function,
            "method" => Self::Method,
            "constructor" => Self::Constructor,
            "destructor" => Self::Destructor,
            "concept" => Self::Concept,
            "class" => Self::Class,
            "struct" => Self::Struct,
            "union" => Self::Union,
            "interface" => Self::Interface,
            "trait" => Self::Trait,
            "enum" => Self::Enum,
            "enum_member" => Self::EnumMember,
            "type" => Self::Type,
            "type_alias" => Self::TypeAlias,
            "variable" => Self::Variable,
            "constant" => Self::Constant,
            "field" => Self::Field,
            "property" => Self::Property,
            "module" => Self::Module,
            "namespace" => Self::Namespace,
            "impl" => Self::Impl,
            "macro" => Self::Macro,
            "component" => Self::Component,
            "object" => Self::Object,
            "record" => Self::Record,
            _ => Self::Unknown,
        }
    }
    pub fn callable(self) -> bool {
        matches!(
            self,
            Self::Function | Self::Method | Self::Constructor | Self::Destructor
        )
    }
    pub fn is_type(self) -> bool {
        matches!(
            self,
            Self::Class
                | Self::Concept
                | Self::Struct
                | Self::Union
                | Self::Interface
                | Self::Trait
                | Self::Enum
                | Self::TypeAlias
                | Self::Type
                | Self::Object
                | Self::Record
                | Self::Component
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Modifiers {
    pub is_static: Option<bool>,
    pub is_async: Option<bool>,
    pub is_abstract: Option<bool>,
    pub is_mutable: Option<bool>,
    pub is_virtual: Option<bool>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Parameter {
    pub name: Option<String>,
    pub declared_type: Option<String>,
    pub optional: bool,
    pub variadic: bool,
    pub receiver: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeRelation {
    pub kind: RelationKind,
    pub target: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SymbolFacts {
    pub kind: SymbolKind,
    pub role: SymbolRole,
    pub scope: ScopeKind,
    pub qualified_name: String,
    pub owner: Option<String>,
    pub package: Option<String>,
    pub name_range: Option<[usize; 2]>,
    pub visibility: Visibility,
    pub modifiers: Modifiers,
    pub declared_type: Option<String>,
    pub inferred_type: Option<String>,
    pub parameters: Option<Vec<Parameter>>,
    pub return_type: Option<String>,
    pub type_parameters: Vec<String>,
    pub extension_receiver: Option<String>,
    pub relations: Vec<TypeRelation>,
    pub language_details: std::collections::BTreeMap<String, serde_json::Value>,
}
impl SymbolFacts {
    pub fn accepts_arity(&self, count: i64) -> bool {
        let Some(params) = &self.parameters else {
            return true;
        };
        let params: Vec<_> = params.iter().filter(|p| !p.receiver).collect();
        let required = params.iter().filter(|p| !p.optional && !p.variadic).count() as i64;
        count >= required && (params.iter().any(|p| p.variadic) || count <= params.len() as i64)
    }
}

/// Syntax-level policy, deliberately separate from compiler-dependent typing.
pub struct LanguageRules {
    pub implicit_members: bool,
    pub separate_type_namespace: bool,
    pub declarations_hoisted: bool,
}
pub fn rules(language: &str) -> LanguageRules {
    LanguageRules {
        implicit_members: matches!(language, "java" | "kotlin" | "cpp"),
        separate_type_namespace: matches!(language, "rust" | "typescript" | "vue" | "svelte"),
        declarations_hoisted: matches!(
            language,
            "rust" | "go" | "java" | "kotlin" | "javascript" | "typescript" | "vue" | "svelte"
        ),
    }
}
