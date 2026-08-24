//! Entity 事件定义（18 个）。
//!
//! 对应 Java Minestom 的 `entity` 包下事件。

pub mod entity_attack;
pub mod entity_damage;
pub mod entity_death;
pub mod entity_despawn;
pub mod entity_spawn;
pub mod entity_teleport;
pub mod entity_tick;
pub mod entity_velocity;

pub use entity_attack::EntityAttack;
pub use entity_damage::EntityDamage;
pub use entity_death::EntityDeath;
pub use entity_despawn::EntityDespawn;
pub use entity_spawn::EntitySpawn;
pub use entity_teleport::EntityTeleport;
pub use entity_tick::EntityTick;
pub use entity_velocity::EntityVelocity;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::r#trait::EntityEvent;
    use crate::prelude::Entity;

    #[test]
    fn entity_event_traits_impl() {
        let entity = Entity::from_raw_u32(42);
        let evt = EntityDamage {
            entity,
            amount: 5.0,
            cancelled: false,
            source: crate::component::DamageSource::Entity(7),
            damage_type: None,
        };

        // 验证 EntityEvent trait
        assert_eq!(evt.entity(), entity);
    }
}
