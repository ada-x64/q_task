use bevy::{
    ecs::world::CommandQueue,
    prelude::*,
    tasks::{block_on, futures_lite::future, Task},
};

#[derive(Component)]
pub struct TaskComponent(pub Task<CommandQueue>);

fn poll_tasks(mut commands: Commands, tasks: Query<&mut TaskComponent>) {
    for mut task in tasks {
        if let Some(mut q) = block_on(future::poll_once(&mut task.0)) {
            commands.append(&mut q);
        }
    }
}

/// Arguments: an async move lambda taking &mut CommandQueue as argument.
///
/// Example usage:
/// ```rust, ignore
/// task!(async move |_q| { info!("do something") })(world)
/// ```
#[macro_export]
macro_rules! task {
    ($pool_type:path, $block:expr) => {
        (move |world: &mut World| {
            let mut entity = world.spawn_empty();
            let id = entity.id();
            let task = <$pool_type>::get().spawn(async move {
                let mut q = bevy::ecs::world::CommandQueue::default();
                ($block)(&mut q).await;
                q.push(move |world: &mut World| {
                    world.despawn(id);
                });
                q
            });
            entity.insert($crate::TaskComponent(task));
        })
    };
}

pub struct TaskPlugin;
impl Plugin for TaskPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, poll_tasks);
    }
}

#[test]
fn test() {
    let mut app = App::new();
    let i = 0;
    app.add_plugins((MinimalPlugins, TaskPlugin))
        .add_systems(Update, move |world: &mut World| {
            task!(bevy::tasks::IoTaskPool, async move |_q| { println!("{i}") })(world)
        });
}
