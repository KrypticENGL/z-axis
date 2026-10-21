//! The only place `unsafe` and raw Win32 live. Everything outside `sys` is safe Rust.
#![allow(unsafe_code)]

pub mod d3d;
pub mod win;
