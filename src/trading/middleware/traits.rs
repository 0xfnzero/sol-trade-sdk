use anyhow::Result;
use solana_sdk::instruction::Instruction;

/// Instruction middleware trait
///
/// Used to modify, add or remove protocol_instructions before transaction execution
pub trait InstructionMiddleware: Send + Sync {
    /// Middleware name
    fn name(&self) -> &'static str;

    /// Core method for processing protocol_instructions
    ///
    /// # Arguments
    /// * `protocol_instructions` - Current instruction list
    /// * `protocol_name` - Protocol name
    /// * `is_buy` - Whether the transaction is a buy transaction
    ///
    /// # Returns
    /// Returns modified instruction list
    fn process_protocol_instructions(
        &self,
        protocol_instructions: Vec<Instruction>,
        protocol_name: &str,
        is_buy: bool,
    ) -> Result<Vec<Instruction>>;

    /// Core method for processing full_instructions
    ///
    /// # Arguments
    /// * `full_instructions` - Current instruction list
    /// * `protocol_name` - Protocol name
    /// * `is_buy` - Whether the transaction is a buy transaction
    ///
    /// # Returns
    /// Returns modified instruction list
    fn process_full_instructions(
        &self,
        full_instructions: Vec<Instruction>,
        protocol_name: &str,
        is_buy: bool,
    ) -> Result<Vec<Instruction>>;

    /// Clone middleware
    fn clone_box(&self) -> Box<dyn InstructionMiddleware>;
}

/// Middleware manager
pub struct MiddlewareManager {
    middlewares: Vec<Box<dyn InstructionMiddleware>>,
}

impl Clone for MiddlewareManager {
    fn clone(&self) -> Self {
        Self {
            middlewares: self.middlewares.iter().map(|middleware| middleware.clone_box()).collect(),
        }
    }
}

impl MiddlewareManager {
    /// Create new middleware manager
    pub fn new() -> Self {
        Self { middlewares: Vec::new() }
    }

    /// Add middleware
    pub fn add_middleware(mut self, middleware: Box<dyn InstructionMiddleware>) -> Self {
        self.middlewares.push(middleware);
        self
    }

    pub fn apply_middlewares_process_full_instructions(
        &self,
        mut full_instructions: Vec<Instruction>,
        protocol_name: &str,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        for middleware in &self.middlewares {
            full_instructions =
                middleware.process_full_instructions(full_instructions, protocol_name, is_buy)?;
            if full_instructions.is_empty() {
                break;
            }
        }
        Ok(full_instructions)
    }

    /// Apply all middlewares to process protocol_instructions
    pub fn apply_middlewares_process_protocol_instructions(
        &self,
        mut protocol_instructions: Vec<Instruction>,
        protocol_name: &str,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        for middleware in &self.middlewares {
            protocol_instructions = middleware.process_protocol_instructions(
                protocol_instructions,
                protocol_name,
                is_buy,
            )?;
            if protocol_instructions.is_empty() {
                break;
            }
        }
        Ok(protocol_instructions)
    }

    /// Create manager with common middlewares
    pub fn with_common_middlewares() -> Self {
        Self::new().add_middleware(Box::new(crate::trading::middleware::builtin::LoggingMiddleware))
    }
}
