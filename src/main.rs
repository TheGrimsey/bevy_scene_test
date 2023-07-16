use bevy::{prelude::*, ecs::system::SystemState, reflect::{TypeUuid, TypeRegistryArc, TypePath, TypeRegistryInternal}, asset::{AssetLoader, LoadContext, LoadedAsset}, utils::BoxedFuture, scene::serde::{SceneEntitiesDeserializer, EntitiesSerializer}};
use serde::{Serialize, Deserialize, de::{DeserializeSeed, Visitor}, ser::SerializeStruct};
use anyhow::anyhow;

fn main() {
    let mut app = App::new();

    app.add_plugins(DefaultPlugins);

    app.add_systems(Startup, spawn_world_system);
    app.add_systems(PostStartup, serialize_world_system);

    app.init_asset_loader::<PrefabLoader>();

    app.run();
}

#[derive(Resource)]
struct SceneToSave(Entity);

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct TestComponent {
    name: String
}

#[derive(Debug, Serialize, Deserialize)]
struct SerializedPrefab {
    name: String,
    scene: String
}

#[derive(Debug, Default)]
pub struct PrefabLoader {
    type_registry: TypeRegistryArc,
}
impl AssetLoader for PrefabLoader {
    fn load<'a>(
        &'a self,
        bytes: &'a [u8],
        load_context: &'a mut LoadContext,
    ) -> BoxedFuture<'a, Result<(), bevy::asset::Error>> {
        Box::pin(async move {
            let mut deserializer = ron::de::Deserializer::from_bytes(bytes)?;
            
            let prefab_deserializer = PrefabDeserializer {
                type_registry: &self.type_registry
            };
            let prefab = prefab_deserializer.deserialize(&mut deserializer).map_err(|e| {
                let span_error = deserializer.span_error(e);
                anyhow!(
                    "{} at {}:{}",
                    span_error.code,
                    load_context.path().to_string_lossy(),
                    span_error.position,
                )
            })?;

            load_context.set_default_asset(LoadedAsset::new(prefab));
            Ok(())
        })
    }

    fn extensions(&self) -> &[&str] {
        &["prefab"]
    }
}

#[derive(TypeUuid, TypePath)]
#[uuid = "09433411-5448-4168-970e-02341c20e9ed"]
struct Prefab {
    name: String,
    scene: DynamicScene
}

struct PrefabSerializer<'a> {
    prefab: &'a Prefab,
    pub registry: &'a TypeRegistryArc,
}
impl<'a> Serialize for PrefabSerializer<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer {
        
        let mut state = serializer.serialize_struct("Prefab", 2)?;

        state.serialize_field("name", &self.prefab.name)?;
        
        state.serialize_field(
            "scene",
            &EntitiesSerializer {
                entities: &self.prefab.scene.entities,
                registry: self.registry,
            },
        )?;

        state.end()
    }
}
struct PrefabDeserializer<'a> {
    type_registry: &'a TypeRegistryArc,
}
impl<'a, 'de> DeserializeSeed<'de> for PrefabDeserializer<'a> {
    type Value = Prefab;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let type_registry = self.type_registry.internal.read();

        let prefab = deserializer.deserialize_struct(
            "Prefab",
            &["name", "scene"],
            PrefabVisitor {
                type_registry: &type_registry,
            },
        )?;

        Ok(prefab)
    }
}

struct PrefabVisitor<'a> {
    pub type_registry: &'a TypeRegistryInternal,
}

impl<'a, 'de> Visitor<'de> for PrefabVisitor<'a> {
    type Value = Prefab;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Prefab Struct")
    }
    
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>, {
        
        let name = seq.next_element()?.ok_or_else(|| serde::de::Error::missing_field("Name"))?;

        let entities = seq.next_element_seed(SceneEntitiesDeserializer {
            type_registry: self.type_registry
        })?.ok_or_else(|| serde::de::Error::missing_field("Scene"))?;
        
        let scene = DynamicScene { 
            resources: Vec::default(),
            entities
        };

        Ok(Prefab { 
            name,
            scene
        })
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct PrefabMarker;

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct LeafNode;

fn spawn_world_system(
    mut commands: Commands
) {
    let scene = commands.spawn((
        TestComponent {
            name: "Steve".to_owned()
        },
        TransformBundle::default()
    )).with_children(|child_builer| {
        child_builer.spawn((
            TestComponent {
                name: "Stove".to_owned()
            },
            TransformBundle::from_transform(Transform::from_xyz(1.0, 0.5, -1.3)),
            LeafNode
        )).with_children(|child_builder| {
            child_builder.spawn(TransformBundle::from_transform(Transform::from_xyz(0.0, 5.0, 0.0)));
        });
    }).id();

    commands.insert_resource(SceneToSave(scene));
}

fn serialize_world_system(
    world: &mut World
) {
    let entity_to_save = world.resource::<SceneToSave>().0;
    
    let (entities, leaf_nodes) = {
        let mut entities = vec![entity_to_save];
        let mut leaf_nodes = Vec::new();

        let mut system_state = SystemState::<(Query<&Children>, Query<(), With<LeafNode>>)>::new(world);

        let (child_query, is_leaf_node)  = system_state.get(world);

        let mut entities_to_check = child_query.get(entity_to_save).map(|val| val.iter().cloned().collect()).unwrap_or_else(|_| Vec::default());

        while let Some(entity) = entities_to_check.pop() {
            if is_leaf_node.contains(entity) {
                leaf_nodes.push(entity);
            } else {
                entities.push(entity);

                if let Ok(children) = child_query.get(entity) {
                    entities_to_check.extend(children.iter().cloned());
                }
            }
        }

        (entities, leaf_nodes)
    };
    
    let mut scene_builder = DynamicSceneBuilder::from_world(world);


    scene_builder
        .deny::<GlobalTransform>()
        .extract_entities(entities.into_iter());

    scene_builder.deny::<Children>()
        .extract_entities(leaf_nodes.into_iter());

    let scene = scene_builder.build();
    
    let type_registry = world.resource::<AppTypeRegistry>();

    let prefab = Prefab {
        name: "Test".to_owned(),
        scene
    };

    let prefab_serializer = PrefabSerializer {
        prefab: &prefab,
        registry: type_registry
    };

    let pretty_config = ron::ser::PrettyConfig::default()
        .indentor("  ".to_string())
        .new_line("\n".to_string());
    let serialized_prefab = ron::ser::to_string_pretty(&prefab_serializer, pretty_config).unwrap();
    info!("Serialized: {serialized_prefab}");
}