use bevy_app::prelude::*;
use bevy_ecs::prelude::*;
pub use bevy_ecs::world::{CommandQueue, World};
use bevy_tasks::{Task, futures_lite::future, prelude::*};

#[derive(Component)]
pub struct TaskComponent(pub Task<CommandQueue>);

fn poll_tasks(mut commands: Commands, tasks: Query<&mut TaskComponent>) {
    for mut task in tasks {
        if let Some(mut q) = block_on(future::poll_once(&mut task.0)) {
            commands.append(&mut q);
        }
    }
}

/// Creates an asychronously executing task. When finished, may optionally send an event.
///
/// Example usage:
/// ```rust
/// # use bevy_app::prelude::*;
/// # use bevy_tasks::IoTaskPool;
/// # use bevy_app::{ScheduleRunnerPlugin, TaskPoolPlugin};
/// # use bevy_ecs::prelude::*;
/// # use q_tasks::*;
/// #
/// # #[derive(Event, Default)]
/// # pub struct MyEvent();
/// #
/// # let mut app = App::new();
/// # app.add_plugins((
/// #     TaskPoolPlugin::default(),
/// #     ScheduleRunnerPlugin::default(),
/// #     TaskPlugin,
/// #  ));
/// # let mut world = app.world_mut();
/// let task = task!(
///     IoTaskPool,
///     MyEvent::default(), // (optional)
///     async move |q: &mut CommandQueue| {
///         // do some async stuff
///         q.push(|world: &mut World| {
///             // do some world mutation
///         });
///     });
/// task(world);
/// ```
#[macro_export]
macro_rules! task {
    ($pool_type:path, $block:expr) => {
        task!(@inner $pool_type, $block)
    };
    ($pool_type:path, $event:expr, $block:expr) => {
        task!(@inner $pool_type, $block, $event)
    };
    (@inner $pool_type:path, $block:expr $(, $event:expr)?)  => {
        (move |world: &mut $crate::World| {
            let mut entity = world.spawn_empty();
            let id = entity.id();
            let task = <$pool_type>::get().spawn(async move {
                let mut q = $crate::CommandQueue::default();
                ($block)(&mut q).await;
                q.push(move |world: &mut $crate::World| {
                    world.despawn(id);
                    $(world.trigger($event))?
                });
                q
            });
            entity.insert($crate::TaskComponent(task));
        })
    }
}

pub struct TaskPlugin;
impl Plugin for TaskPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, poll_tasks);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use bevy::platform::time::Instant;
    use bevy::prelude::*;
    use bevy::{app::ScheduleRunnerPlugin, log::LogPlugin};
    use std::{marker::PhantomData, time::Duration};

    #[derive(Event)]
    struct Ran<T>(PhantomData<T>);
    impl<T> Default for Ran<T> {
        fn default() -> Self {
            Self(PhantomData)
        }
    }

    #[derive(Resource, Default, PartialEq, Debug)]
    struct TestResults {
        io_task_pool: bool,
        io_task_pool_observer: bool,
        compute_task_pool: bool,
        compute_task_pool_observer: bool,
        async_task_pool: bool,
        async_task_pool_observer: bool,
    }

    #[test]
    fn test_io() {
        let mut app = App::new();
        app.add_plugins((
            TaskPoolPlugin::default(),
            ScheduleRunnerPlugin::default(),
            TaskPlugin,
            LogPlugin {
                filter: "debug".to_string(),
                ..Default::default()
            },
        ))
        .init_resource::<TestResults>()
        .add_event::<Ran<IoTaskPool>>()
        .add_event::<Ran<ComputeTaskPool>>()
        .add_event::<Ran<AsyncComputeTaskPool>>()
        .add_systems(Startup, move |world: &mut World| {
            task!(
                ComputeTaskPool,
                Ran::<IoTaskPool>::default(),
                async move |q: &mut CommandQueue| {
                    debug!("In IoTaskPool");
                    q.push(|world: &mut World| {
                        world.resource_mut::<TestResults>().io_task_pool = true;
                    })
                }
            )(world);
            task!(
                ComputeTaskPool,
                Ran::<ComputeTaskPool>::default(),
                async move |q: &mut CommandQueue| {
                    debug!("In ComputeTaskPool");
                    q.push(|world: &mut World| {
                        world.resource_mut::<TestResults>().compute_task_pool = true;
                    })
                }
            )(world);
            task!(
                AsyncComputeTaskPool,
                Ran::<AsyncComputeTaskPool>::default(),
                async move |q: &mut CommandQueue| {
                    // busy-wait 1 sec to test it works
                    debug!("In AsyncComputeTaskPool");
                    let start = Instant::now();
                    while Instant::now().duration_since(start) <= Duration::from_secs(1) {}
                    debug!("...AsyncComputeTaskPool DONE");
                    q.push(|world: &mut World| {
                        world.resource_mut::<TestResults>().async_task_pool = true;
                    })
                }
            )(world);
        })
        .add_observer(
            |_: Trigger<Ran<IoTaskPool>>, mut res: ResMut<TestResults>| {
                res.io_task_pool_observer = true;
            },
        )
        .add_observer(
            |_: Trigger<Ran<ComputeTaskPool>>, mut res: ResMut<TestResults>| {
                res.compute_task_pool_observer = true;
            },
        )
        .add_observer(
            |_: Trigger<Ran<AsyncComputeTaskPool>>, mut res: ResMut<TestResults>| {
                res.async_task_pool_observer = true;
            },
        );
        app.update();
        let res = app
            .world_mut()
            .get_resource::<TestResults>()
            .expect("TestResults");
        assert_eq!(
            *res,
            TestResults {
                io_task_pool: true,
                io_task_pool_observer: true,
                compute_task_pool: true,
                compute_task_pool_observer: true,
                async_task_pool: false,
                async_task_pool_observer: false,
            }
        );
        // Observers and the entity for polling the async compute task.
        assert_eq!(app.world_mut().entities().used_count(), 4);
        // busy wait...
        let start = Instant::now();
        while Instant::now().duration_since(start) <= Duration::from_secs(2) {}
        // ... update again
        app.update();
        let res = app
            .world_mut()
            .get_resource::<TestResults>()
            .expect("TestResults");
        assert_eq!(
            *res,
            TestResults {
                io_task_pool: true,
                io_task_pool_observer: true,
                compute_task_pool: true,
                compute_task_pool_observer: true,
                async_task_pool: true,
                async_task_pool_observer: true,
            }
        );
        // Should only contain the observers.
        assert_eq!(app.world_mut().entities().used_count(), 3);
    }
}
