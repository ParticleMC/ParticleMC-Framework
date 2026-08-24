//! 资源层：Minestom 的 Manager 类 `Resource` 与各类注册表。
//!
//! 对应 Minestom 的 `ConnectionManager` / `InstanceManager` / `CommandManager`
//! / `TaskScheduler`，以及方块 / 物品 / 实体类型等注册数据。除注册表外，
//! 这些 Manager 以 `Resource` 形式提供框架 API（见
//! `.specs/implement-framework-capabilities/`）。

pub mod advancement;
pub mod attribute;
pub mod block_handler;
pub mod boss_bar;
pub mod command;
pub mod compression_config;
pub mod connection_manager;
pub mod damage_type;
pub mod dialog;
pub mod enchantment;
pub mod entity_spawner;
pub mod entity_type;
pub mod instance_manager;
pub mod item_behavior;
pub mod item_handler;
pub mod map;
pub mod message;
pub mod ping;
pub mod potion;
pub mod recipe;
pub mod registries;
pub mod scheduler_manager;
pub mod scoreboard;
pub mod snapshot;
pub mod spawn_config;
pub mod statistic;
pub mod status_config;
pub mod tag;
pub mod timeline;
pub mod timer;
pub mod velocity_config;

pub use advancement::{Advancement, AdvancementError, AdvancementManager};
pub use attribute::{
    Attribute, AttributeInstance, AttributeModifier, AttributeOperation, AttributeRegistry,
};
pub use block_handler::{
    BlockBreakContext, BlockHandler, BlockHandlers, BlockInteractContext, BlockPlaceContext,
};
pub use boss_bar::{BossBar, BossBarColor, BossBarDivision, BossBarError, BossBarManager};
pub use command::CommandManager;
pub use compression_config::CompressionConfig;
pub use connection_manager::ConnectionManager;
pub use damage_type::{
    DamageType, DamageTypeRegistry, EntityDamage, EntityProjectileDamage, PositionalDamage,
};
pub use dialog::{DialogManager, DialogOption, DialogTree};
pub use enchantment::{Enchantment, EnchantmentList, EnchantmentRegistry};
pub use entity_spawner::EntitySpawner;
pub use entity_type::EntityType;
pub use instance_manager::InstanceManager;
pub use item_behavior::{Armor, Crossbow, Tool, Weapon, WritableBook, WrittenBook};
pub use item_handler::{ItemHandler, ItemHandlerRegistry};
pub use map::MapData;
pub use message::{ChatType, ChatTypeRegistry, Messenger};
pub use ping::{ServerListPing, Status};
pub use potion::{PotionEffect, PotionEffects, PotionType, TimedPotion};
pub use recipe::{
    Ingredient, Recipe, RecipeDisplay, RecipeError, RecipeManager, RecipeProperty, SlotDisplay,
    StonecutterRecipe,
};
pub use registries::{
    BiomeRegistry, BlockDefinition, BlockRegistry, BlockStateDef, DimensionTypeRegistry,
    EntityTypeDefinition, EntityTypeRegistry, FluidRegistry, GenericDefinition, GenericRegistry,
    ItemDefinition, ItemRegistry, LootTable, LootTableRegistry, ParticleRegistry,
    PotionEffectRegistry, Registry, RegistryError, SoundEventRegistry, TagRegistry,
};
pub use scheduler_manager::{TaskId, TaskScheduler};
pub use scoreboard::{Objective, ScoreEntry, Scoreboard, ScoreboardError, Team};
pub use snapshot::{ChunkSnapshot, EntitySnapshot, InstanceSnapshot, Snapshotable};
pub use spawn_config::SpawnConfig;
pub use statistic::{PlayerStatistics, Statistic, StatisticRegistry};
pub use status_config::StatusConfig;
pub use tag::{Tag, TagError, TagHandler, TagSerializer, Taggable};
pub use timeline::{Keyframe, Timeline};
pub use timer::{Schedulable, Timer};
pub use velocity_config::VelocityConfig;
