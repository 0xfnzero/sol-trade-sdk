use crate::trading::middleware::traits::InstructionMiddleware;
use anyhow::Result;
use solana_sdk::instruction::Instruction;

/// Logging middleware - Records instruction information
#[derive(Clone)]
pub struct LoggingMiddleware;

impl InstructionMiddleware for LoggingMiddleware {
    fn name(&self) -> &'static str {
        "LoggingMiddleware"
    }

    fn process_protocol_instructions(
        &self,
        protocol_instructions: Vec<Instruction>,
        protocol_name: &str,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        println!("-------------------[{}]-------------------", self.name());
        println!("process_protocol_instructions");
        println!("[{}] Instruction count: {}", self.name(), protocol_instructions.len());
        println!("[{}] Protocol name: {}\n", self.name(), protocol_name);
        println!("[{}] Is buy: {}", self.name(), is_buy);
        for (i, instruction) in protocol_instructions.iter().enumerate() {
            println!("Instruction {}:", i + 1);
            println!("{:?}\n", instruction);
        }
        Ok(protocol_instructions)
    }

    fn process_full_instructions(
        &self,
        full_instructions: Vec<Instruction>,
        protocol_name: &str,
        is_buy: bool,
    ) -> Result<Vec<Instruction>> {
        println!("-------------------[{}]-------------------", self.name());
        println!("process_full_instructions");
        println!("[{}] Instruction count: {}", self.name(), full_instructions.len());
        println!("[{}] Protocol name: {}\n", self.name(), protocol_name);
        println!("[{}] Is buy: {}", self.name(), is_buy);
        for (i, instruction) in full_instructions.iter().enumerate() {
            println!("Instruction {}:", i + 1);
            println!("{:?}\n", instruction);
        }
        Ok(full_instructions)
    }

    fn clone_box(&self) -> Box<dyn InstructionMiddleware> {
        Box::new(self.clone())
    }
}
