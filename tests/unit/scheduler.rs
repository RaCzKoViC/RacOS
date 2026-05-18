//! Unit tests for scheduler

#[cfg(test)]
mod tests {
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum TaskState {
        Ready,
        Running,
        Blocked,
    }

    #[derive(Debug)]
    struct MockTask {
        id: u32,
        state: TaskState,
    }

    struct MockScheduler {
        tasks: Vec<MockTask>,
        current: usize,
    }

    impl MockScheduler {
        fn new() -> Self {
            MockScheduler {
                tasks: Vec::new(),
                current: 0,
            }
        }

        fn add_task(&mut self, id: u32) {
            self.tasks.push(MockTask {
                id,
                state: TaskState::Ready,
            });
        }

        fn schedule_next(&mut self) -> Option<u32> {
            if self.tasks.is_empty() {
                return None;
            }

            // Simple round-robin
            self.current = (self.current + 1) % self.tasks.len();
            Some(self.tasks[self.current].id)
        }
    }

    #[test]
    fn test_scheduler_basic() {
        let mut scheduler = MockScheduler::new();
        
        scheduler.add_task(1);
        scheduler.add_task(2);
        scheduler.add_task(3);

        assert_eq!(scheduler.tasks.len(), 3);
        assert_eq!(scheduler.tasks[0].id, 1);
        assert_eq!(scheduler.tasks[1].id, 2);
        assert_eq!(scheduler.tasks[2].id, 3);
    }

    #[test]
    fn test_scheduler_round_robin() {
        let mut scheduler = MockScheduler::new();
        
        scheduler.add_task(1);
        scheduler.add_task(2);
        scheduler.add_task(3);

        // Initial current is 0
        assert_eq!(scheduler.current, 0);

        // First schedule should go to task 1 (index 1)
        let next = scheduler.schedule_next();
        assert_eq!(next, Some(2)); // tasks[1].id = 2
        assert_eq!(scheduler.current, 1);

        // Next schedule
        let next = scheduler.schedule_next();
        assert_eq!(next, Some(3)); // tasks[2].id = 3
        assert_eq!(scheduler.current, 2);

        // Wrap around
        let next = scheduler.schedule_next();
        assert_eq!(next, Some(1)); // tasks[0].id = 1
        assert_eq!(scheduler.current, 0);
    }

    #[test]
    fn test_scheduler_empty() {
        let mut scheduler = MockScheduler::new();
        
        let next = scheduler.schedule_next();
        assert_eq!(next, None);
    }
}