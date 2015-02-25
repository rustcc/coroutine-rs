#![feature(path)]
extern crate gcc;

use std::path::Path;

const PATH: &'static str = "src/asm";
const ASM_FILE: &'static str = "_context.S";
const LIB_NAME: &'static str = "libctxswtch.a";

fn main() {
    compile();
}

#[cfg(target_arch="x86_64")]
fn compile() {
    gcc::compile_library(LIB_NAME, &[Path::new(&format!("{path}/{arch}/{asm_file}", path = PATH, arch = "x86_64", asm_file = ASM_FILE)).to_str().unwrap()]);
}
    
#[cfg(target_arch="x86")]
fn compile() {
    gcc::compile_library(LIB_NAME, &[Path::new(&format!("{path}/{arch}/{asm_file}", path = PATH, arch = "i686", asm_file = ASM_FILE)).to_str().unwrap()]);
}
    
#[cfg(target_arch="arm")]
fn compile() {
    gcc::compile_library(LIB_NAME, &[Path::new(&format!("{path}/{arch}/{asm_file}", path = PATH, arch = "arm", asm_file = ASM_FILE)).to_str().unwrap()]);
}
    
#[cfg(target_arch="mips")]
fn compile() {
    gcc::compile_library(LIB_NAME, &[Path::new(&format!("{path}/{arch}/{asm_file}", path = PATH, arch = "mips", asm_file = ASM_FILE)).to_str().unwrap()]);
}
    
#[cfg(target_arch="mipsel")]
fn compile() {
    gcc::compile_library(LIB_NAME, &[Path::new(&format!("{path}/{arch}/{asm_file}", path = PATH, arch = "mipsel", asm_file = ASM_FILE)).to_str().unwrap()]);
}
    
