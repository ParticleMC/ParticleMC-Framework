//! 配置阶段数据包（资源包 / 注册表同步 / 完成配置）。

use crate::protocol::byte_buf::ByteBuffer;
use crate::protocol::error::ProtocolError;
use crate::protocol::packet::Packet;

/// 注册表数据（clientbound, id 0x07，wire 名 `registry_data`）。
///
/// 条目值 `value` 为 optional anonymous NBT 的原始字节占位：解码暂不支持非空值
/// （自定界结构需 NBT 解析，由后续任务接线），仅能解码 `None`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryData {
    pub registry_id: String,
    pub entries: Vec<RegistryEntry>,
}

/// 注册表条目：标识 + 可选 NBT 数据（原始字节占位）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub key: String,
    pub value: Option<Vec<u8>>,
}

impl Packet for RegistryData {
    fn packet_id(&self) -> i32 {
        0x07
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let registry_id = buf.get_string()?;
        let count = buf.get_varint()?;
        let count_usize = usize::try_from(count).map_err(|_| ProtocolError::UnexpectedEof)?;
        let mut entries = Vec::with_capacity(count_usize);
        for _ in 0..count_usize {
            let key = buf.get_string()?;
            let value = read_opt_nbt(buf)?;
            entries.push(RegistryEntry { key, value });
        }
        Ok(RegistryData {
            registry_id,
            entries,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.registry_id);
        let count = i32::try_from(self.entries.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
        buf.put_varint(count);
        for entry in &self.entries {
            buf.put_string(&entry.key);
            write_opt_nbt(buf, &entry.value);
        }
        Ok(())
    }
}

/// 更新标签（clientbound, id 0x0d，wire 名 `tags`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateTags {
    pub registries: Vec<TagRegistry>,
}

/// 单个注册表的标签组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRegistry {
    pub registry: String,
    pub tags: Vec<TagEntry>,
}

/// 单个标签：标识符 + 元素 id 列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    pub identifier: String,
    pub entries: Vec<i32>,
}

impl Packet for UpdateTags {
    fn packet_id(&self) -> i32 {
        0x0d
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let registries = read_varint_array(buf, |b| {
            let registry = b.get_string()?;
            let tags = read_varint_array(b, |b| {
                let identifier = b.get_string()?;
                let entries = read_varint_array(b, |b| b.get_varint())?;
                Ok(TagEntry {
                    identifier,
                    entries,
                })
            })?;
            Ok(TagRegistry { registry, tags })
        })?;
        Ok(UpdateTags { registries })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.registries, |b, r| {
            b.put_string(&r.registry);
            write_varint_array(b, &r.tags, |b, t| {
                b.put_string(&t.identifier);
                write_varint_array(b, &t.entries, |b, v| {
                    b.put_varint(*v);
                    Ok(())
                })
            })
        })
    }
}

/// 启用特性（clientbound, id 0x0c，wire 名 `feature_flags`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureFlags {
    pub flags: Vec<String>,
}

impl Packet for FeatureFlags {
    fn packet_id(&self) -> i32 {
        0x0c
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let flags = read_varint_array(buf, |b| b.get_string())?;
        Ok(FeatureFlags { flags })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        write_varint_array(buf, &self.flags, |b, s| {
            b.put_string(s);
            Ok(())
        })
    }
}

/// 读取 optional anonymous NBT 占位：仅支持 `None`，见 [`RegistryData`] 说明。
fn read_opt_nbt(buf: &mut ByteBuffer) -> Result<Option<Vec<u8>>, ProtocolError> {
    if !buf.get_bool()? {
        return Ok(None);
    }
    Err(ProtocolError::UnexpectedEof)
}

/// 写出 optional anonymous NBT 占位：布尔存在位 + 原始字节。
fn write_opt_nbt(buf: &mut ByteBuffer, data: &Option<Vec<u8>>) {
    match data {
        Some(bytes) => {
            buf.put_bool(true);
            buf.put_bytes(bytes);
        }
        None => buf.put_bool(false),
    }
}

/// 读取 varint 计数的 `T` 数组。
fn read_varint_array<T, F>(buf: &mut ByteBuffer, mut item: F) -> Result<Vec<T>, ProtocolError>
where
    F: FnMut(&mut ByteBuffer) -> Result<T, ProtocolError>,
{
    let count = buf.get_varint()?;
    let count_usize = usize::try_from(count).map_err(|_| ProtocolError::UnexpectedEof)?;
    let mut items = Vec::with_capacity(count_usize);
    for _ in 0..count_usize {
        items.push(item(buf)?);
    }
    Ok(items)
}

/// 写入 varint 计数的 `T` 数组。
fn write_varint_array<T, F>(
    buf: &mut ByteBuffer,
    items: &[T],
    mut item: F,
) -> Result<(), ProtocolError>
where
    F: FnMut(&mut ByteBuffer, &T) -> Result<(), ProtocolError>,
{
    let count = i32::try_from(items.len()).map_err(|_| ProtocolError::UnexpectedEof)?;
    buf.put_varint(count);
    for v in items {
        item(buf, v)?;
    }
    Ok(())
}

/// 客户端信息（serverbound, id 0x00）。服务端仅记录、不强制解析全部字段。
#[derive(Debug, Clone, Default)]
pub struct ClientInformation;

impl Packet for ClientInformation {
    fn packet_id(&self) -> i32 {
        0x00
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ClientInformation)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 配置插件消息（serverbound, id 0x02；clientbound 为 0x01）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMessage {
    /// 通道标识（如 `minecraft:brand`）。
    pub channel: String,
    /// 消息体字节。
    pub data: Vec<u8>,
}

impl Packet for PluginMessage {
    fn packet_id(&self) -> i32 {
        0x02
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        let channel = buf.get_string()?;
        let len = buf.get_varint()?;
        let len_usize = usize::try_from(len).map_err(|_| ProtocolError::UnexpectedEof)?;
        let data = buf.get_bytes(len_usize)?;
        Ok(PluginMessage { channel, data })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.channel);
        buf.put_varint(self.data.len() as i32);
        buf.put_bytes(&self.data);
        Ok(())
    }
}

/// 完成配置：客户端 → 服务端（serverbound, id 0x03）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FinishConfigurationC2S;

impl Packet for FinishConfigurationC2S {
    fn packet_id(&self) -> i32 {
        0x03
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(FinishConfigurationC2S)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 完成配置：服务端 → 客户端（clientbound, id 0x03，1.21.11）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FinishConfigurationS2C;

impl Packet for FinishConfigurationS2C {
    fn packet_id(&self) -> i32 {
        0x03
    }
    fn decode(_buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(FinishConfigurationS2C)
    }
    fn encode(&self, _buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        Ok(())
    }
}

/// 配置断开（clientbound, id 0x02，1.21.11）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDisconnect {
    pub reason: String,
}

impl Packet for ConfigDisconnect {
    fn packet_id(&self) -> i32 {
        0x02
    }
    fn decode(buf: &mut ByteBuffer) -> Result<Self, ProtocolError> {
        Ok(ConfigDisconnect {
            reason: buf.get_string()?,
        })
    }
    fn encode(&self, buf: &mut ByteBuffer) -> Result<(), ProtocolError> {
        buf.put_string(&self.reason);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// encode → decode 往返后应与原值一致。
    fn roundtrip<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let mut buf = ByteBuffer::with_capacity(64);
        p.encode(&mut buf).unwrap();
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = P::decode(&mut buf).unwrap();
        assert_eq!(*p, decoded);
    }

    #[test]
    fn plugin_message_roundtrip() {
        // 品牌插件消息往返。
        roundtrip(&PluginMessage {
            channel: "minecraft:brand".to_string(),
            data: b"Paper".to_vec(),
        });
    }

    #[test]
    fn config_disconnect_roundtrip() {
        // reason 为 JSON 聊天组件。
        roundtrip(&ConfigDisconnect {
            reason: r#"{"text":"配置完成"}"#.to_string(),
        });
    }

    #[test]
    fn client_information_empty_roundtrip() {
        // 无字段包：packet_id 正确，空编解码不 panic、往返后仍为同类型。
        // ClientInformation 未实现 PartialEq，故不通过 roundtrip helper。
        assert_eq!(ClientInformation.packet_id(), 0x00);
        let mut buf = ByteBuffer::with_capacity(8);
        ClientInformation.encode(&mut buf).unwrap();
        assert!(buf.is_empty(), "无字段包不应写入任何字节");
        let mut buf = ByteBuffer::new(buf.into_inner());
        let decoded = ClientInformation::decode(&mut buf).unwrap();
        assert!(matches!(decoded, ClientInformation));
    }

    #[test]
    fn finish_configuration_s2c_empty_roundtrip() {
        assert_eq!(FinishConfigurationS2C.packet_id(), 0x03);
        roundtrip(&FinishConfigurationS2C);
    }

    #[test]
    fn finish_configuration_c2s_empty_roundtrip() {
        assert_eq!(FinishConfigurationC2S.packet_id(), 0x03);
        roundtrip(&FinishConfigurationC2S);
    }

    #[test]
    fn registry_data_roundtrip() {
        // 条目值仅使用 None（NBT 占位解码暂不支持 Some）。
        roundtrip(&RegistryData {
            registry_id: "minecraft:dimension_type".to_string(),
            entries: vec![
                RegistryEntry {
                    key: "minecraft:overworld".to_string(),
                    value: None,
                },
                RegistryEntry {
                    key: "minecraft:the_nether".to_string(),
                    value: None,
                },
            ],
        });
    }

    #[test]
    fn registry_data_empty_entries_roundtrip() {
        roundtrip(&RegistryData {
            registry_id: "minecraft:worldgen/biome".to_string(),
            entries: Vec::new(),
        });
    }

    #[test]
    fn update_tags_roundtrip() {
        roundtrip(&UpdateTags {
            registries: vec![
                TagRegistry {
                    registry: "minecraft:block".to_string(),
                    tags: vec![TagEntry {
                        identifier: "minecraft:acacia_logs".to_string(),
                        entries: vec![0, 1, 2],
                    }],
                },
                TagRegistry {
                    registry: "minecraft:item".to_string(),
                    tags: vec![
                        TagEntry {
                            identifier: "minecraft:logs".to_string(),
                            entries: vec![5, 6],
                        },
                        TagEntry {
                            identifier: "minecraft:planks".to_string(),
                            entries: vec![7],
                        },
                    ],
                },
            ],
        });
    }

    #[test]
    fn feature_flags_roundtrip() {
        roundtrip(&FeatureFlags {
            flags: vec![
                "minecraft:update_1_21".to_string(),
                "minecraft:update_1_21_2".to_string(),
            ],
        });
    }

    /// NBT 占位：含 NBT 的注册表条目解码返回 Err（游标无法推进，由后续任务接线）。
    #[test]
    fn registry_data_nbt_placeholder_rejects_some() {
        let mut buf = ByteBuffer::with_capacity(16);
        buf.put_bool(true);
        buf.put_bytes(&[0x0a, 0x00]);
        let mut buf = ByteBuffer::new(buf.into_inner());
        assert_eq!(read_opt_nbt(&mut buf), Err(ProtocolError::UnexpectedEof));
    }
}
