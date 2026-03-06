//! Type Evolution Tracking
//!
//! Manages monotonic type growth where variables can have fields
//! added through assignment operations, with tracking of control
//! flow context for proper optionality.

use super::super::SemanticType;
use std::collections::HashMap;

/// Represents a field that was added to a type through evolution
#[derive(Debug, Clone, PartialEq)]
pub struct EvolvedField {
    /// The name of the field
    pub name: String,
    /// The type of the field
    pub field_type: SemanticType,
    /// Whether this field is optional (added in conditional/loop)
    pub optional: bool,
    /// Control flow depth when this field was added
    pub depth: usize,
}

/// Tracks how a variable's type evolves through the program
#[derive(Debug, Clone)]
pub struct TypeEvolution {
    /// The variable name being tracked
    pub variable: String,
    /// The initial type when the variable was declared
    pub initial_type: SemanticType,
    /// Fields added through assignment operations (monotonic growth)
    pub evolved_fields: Vec<EvolvedField>,
}

impl TypeEvolution {
    /// Create a new type evolution tracker
    pub fn new(variable: String, initial_type: SemanticType) -> Self {
        TypeEvolution {
            variable,
            initial_type,
            evolved_fields: Vec::new(),
        }
    }

    /// Add a new field to the evolution
    pub fn add_field(
        &mut self,
        name: String,
        field_type: SemanticType,
        optional: bool,
        depth: usize,
    ) {
        // Check if field already exists
        if let Some(existing) = self.evolved_fields.iter_mut().find(|f| f.name == name) {
            // If re-assigned at higher depth, mark as optional
            if depth > existing.depth && !existing.optional {
                existing.optional = true;
            }
        } else {
            self.evolved_fields.push(EvolvedField {
                name,
                field_type,
                optional,
                depth,
            });
        }
    }

    /// Get the current type including all evolved fields
    pub fn current_type(&self) -> SemanticType {
        if self.evolved_fields.is_empty() {
            return self.initial_type.clone();
        }

        // For struct types, add evolved fields
        if let SemanticType::Struct { name, fields } = &self.initial_type {
            let mut all_fields = fields.clone();
            for ef in &self.evolved_fields {
                // If optional, wrap in Option
                let field_type = if ef.optional {
                    SemanticType::Option(Box::new(ef.field_type.clone()))
                } else {
                    ef.field_type.clone()
                };
                all_fields.push((ef.name.clone(), field_type));
            }
            SemanticType::Struct {
                name: name.clone(),
                fields: all_fields,
            }
        } else {
            // For non-struct types, evolution doesn't apply
            self.initial_type.clone()
        }
    }

    /// Convert this evolution to a canonical type for JIT compilation
    pub fn to_canonical(&self) -> CanonicalType {
        let mut fields = Vec::new();

        // Add initial fields
        if let SemanticType::Struct {
            fields: initial_fields,
            ..
        } = &self.initial_type
        {
            for (name, field_type) in initial_fields {
                fields.push(CanonicalField::new(
                    name.clone(),
                    field_type.clone(),
                    false, // Initial fields are never optional
                ));
            }
        }

        // Add evolved fields
        for ef in &self.evolved_fields {
            fields.push(CanonicalField::new(
                ef.name.clone(),
                ef.field_type.clone(),
                ef.optional,
            ));
        }

        CanonicalType::new(self.variable.clone(), fields)
    }
}

/// Context for tracking control flow during type evolution
#[derive(Debug, Clone)]
pub struct ControlFlowContext {
    /// Whether we're inside a conditional branch
    pub in_conditional: bool,
    /// Whether we're inside a loop
    pub in_loop: bool,
    /// Nesting depth (for determining field optionality)
    pub depth: usize,
}

/// A canonical field in a finalized type layout
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalField {
    /// The name of the field
    pub name: String,
    /// The type of the field (semantic type)
    pub field_type: SemanticType,
    /// Whether this field is optional (may not be present at runtime)
    pub optional: bool,
    /// Byte offset in the data layout (computed during JIT)
    pub offset: usize,
}

impl CanonicalField {
    pub fn new(name: String, field_type: SemanticType, optional: bool) -> Self {
        CanonicalField {
            name,
            field_type,
            optional,
            offset: 0, // Computed later
        }
    }

    /// Compute and set the byte offset based on previous field end
    pub fn with_offset(mut self, offset: usize) -> Self {
        self.offset = offset;
        self
    }
}

/// A canonical type representing the finalized layout of an evolved type
#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalType {
    /// The name of the type
    pub name: String,
    /// All fields in layout order
    pub fields: Vec<CanonicalField>,
    /// Total data size in bytes (computed from fields)
    pub data_size: usize,
}

impl CanonicalType {
    /// Create a new canonical type from fields
    pub fn new(name: String, fields: Vec<CanonicalField>) -> Self {
        // Compute offsets and total size
        let mut computed_fields = Vec::with_capacity(fields.len());
        let mut offset = 0;

        for field in fields {
            let aligned_offset = (offset + 7) & !7; // 8-byte alignment
            let mut new_field = field;
            new_field.offset = aligned_offset;
            offset = aligned_offset + 8; // All fields are 8 bytes (NaN-boxed)
            computed_fields.push(new_field);
        }

        let data_size = (offset + 7) & !7; // Round up to 8-byte alignment

        CanonicalType {
            name,
            fields: computed_fields,
            data_size,
        }
    }

    /// Get field by name
    pub fn get_field(&self, name: &str) -> Option<&CanonicalField> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Get field offset by name
    pub fn field_offset(&self, name: &str) -> Option<usize> {
        self.get_field(name).map(|f| f.offset)
    }

    /// Check if field is optional
    pub fn is_field_optional(&self, name: &str) -> bool {
        self.get_field(name).map(|f| f.optional).unwrap_or(false)
    }
}

/// Registry for tracking type evolutions
#[derive(Debug, Clone, Default)]
pub struct EvolutionRegistry {
    /// Type evolutions for monotonic growth tracking
    type_evolutions: HashMap<String, TypeEvolution>,
    /// Control flow stack for tracking conditional/loop context
    control_flow_stack: Vec<ControlFlowContext>,
}

impl EvolutionRegistry {
    /// Create a new evolution registry
    pub fn new() -> Self {
        Self {
            type_evolutions: HashMap::new(),
            control_flow_stack: Vec::new(),
        }
    }

    /// Begin tracking type evolution for a variable
    pub fn begin_evolution(&mut self, var_name: &str, initial_type: SemanticType) {
        self.type_evolutions.insert(
            var_name.to_string(),
            TypeEvolution::new(var_name.to_string(), initial_type),
        );
    }

    /// Record a field assignment for type evolution tracking
    pub fn record_field_assignment(
        &mut self,
        var_name: &str,
        field_name: &str,
        field_type: SemanticType,
    ) {
        let depth = self.control_flow_stack.len();
        let is_conditional = self
            .control_flow_stack
            .iter()
            .any(|ctx| ctx.in_conditional || ctx.in_loop);

        if let Some(evolution) = self.type_evolutions.get_mut(var_name) {
            evolution.add_field(field_name.to_string(), field_type, is_conditional, depth);
        }
    }

    /// Get the current evolved type for a variable
    pub fn get_evolved_type(&self, var_name: &str) -> Option<SemanticType> {
        self.type_evolutions
            .get(var_name)
            .map(|ev| ev.current_type())
    }

    /// Get the type evolution for a variable
    pub fn get_evolution(&self, var_name: &str) -> Option<&TypeEvolution> {
        self.type_evolutions.get(var_name)
    }

    /// Enter a conditional block (if/else)
    pub fn enter_conditional(&mut self) {
        self.control_flow_stack.push(ControlFlowContext {
            in_conditional: true,
            in_loop: false,
            depth: self.control_flow_stack.len(),
        });
    }

    /// Exit a conditional block
    pub fn exit_conditional(&mut self) {
        self.control_flow_stack.pop();
    }

    /// Enter a loop block (for/while)
    pub fn enter_loop(&mut self) {
        self.control_flow_stack.push(ControlFlowContext {
            in_conditional: false,
            in_loop: true,
            depth: self.control_flow_stack.len(),
        });
    }

    /// Exit a loop block
    pub fn exit_loop(&mut self) {
        self.control_flow_stack.pop();
    }

    /// Check if we're inside a conditional or loop context
    pub fn in_conditional_context(&self) -> bool {
        self.control_flow_stack
            .iter()
            .any(|ctx| ctx.in_conditional || ctx.in_loop)
    }

    /// Get all type evolutions
    pub fn all_evolutions(&self) -> &HashMap<String, TypeEvolution> {
        &self.type_evolutions
    }
}
