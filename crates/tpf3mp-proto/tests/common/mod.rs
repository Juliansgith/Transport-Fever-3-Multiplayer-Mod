//! Shared by the tests that run the Lua mod.

#![allow(clippy::unwrap_used, dead_code)]

use mlua::Value;
use tpf3mp_proto::lua::LuaValue;

/// A Lua value as the hook reads it from the game's Lua state.
pub fn tree(value: &Value) -> LuaValue {
    match value {
        Value::Nil => LuaValue::Nil,
        Value::Boolean(b) => LuaValue::Boolean(*b),
        Value::Integer(n) => LuaValue::Integer(*n),
        Value::Number(n) => LuaValue::Number(*n),
        Value::String(s) => LuaValue::String(s.as_bytes().to_vec()),
        Value::Table(t) => LuaValue::Table(
            t.pairs::<Value, Value>()
                .map(|pair| {
                    let (k, v) = pair.unwrap();
                    (tree(&k), tree(&v))
                })
                .collect(),
        ),
        other => panic!("not an action table: {other:?}"),
    }
}

/// A Lua value as the hook hands it to the game's Lua state.
pub fn value(lua: &mlua::Lua, tree: &LuaValue) -> Value {
    match tree {
        LuaValue::Nil => Value::Nil,
        LuaValue::Boolean(b) => Value::Boolean(*b),
        LuaValue::Integer(n) => Value::Integer(*n),
        LuaValue::Number(n) => Value::Number(*n),
        LuaValue::String(s) => Value::String(lua.create_string(s).unwrap()),
        LuaValue::Table(pairs) => {
            let t = lua.create_table().unwrap();
            for (k, v) in pairs {
                t.raw_set(value(lua, k), value(lua, v)).unwrap();
            }
            Value::Table(t)
        }
    }
}
