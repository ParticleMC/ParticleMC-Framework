//! Instance 事件定义（8 个）。

pub mod instance_chunk_load;
pub mod instance_chunk_unload;
pub mod instance_register;
pub mod instance_tick;
pub mod instance_unregister;

pub use instance_chunk_load::InstanceChunkLoad;
pub use instance_chunk_unload::InstanceChunkUnload;
pub use instance_register::InstanceRegister;
pub use instance_tick::InstanceTick;
pub use instance_unregister::InstanceUnregister;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::r#trait::InstanceEvent;
    use particlemc_framework_ecs::scheduler::WorldId;

    #[test]
    fn instance_event_traits_impl() {
        let evt = InstanceRegister {
            instance_id: WorldId(1),
        };
        assert_eq!(evt.instance_id(), Some(WorldId(1)));
    }
}
