// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: Copyright (c) 2025 Asymptotic Inc.
// SPDX-FileCopyrightText: Copyright (c) 2025 Arun Raghavan

use crate::properties;

pub fn read_env_bool(var: &str, default: bool) -> bool {
    std::env::var(var)
        .map(|v| properties::parse_bool(&v))
        .unwrap_or(default)
}

pub fn read_env_string(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or(default.to_string())
}
