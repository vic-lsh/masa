from typing import List, Optional, Tuple

Timestamp = int
Duration = int
Error = str


def time_now() -> Timestamp:
    raise NotImplementedError


class Task:
    # [TODO] This requires changes to the async future metadata.
    # Estimated task execution interval
    # `[latest_exec, deadline = latest_exec + future_work_time]`.
    latest_exec: Timestamp
    deadline: Timestamp
    future_work_time: Duration
    # Whether the task is admitted to the present queue.
    admitted: bool
    # Latest time when the task was popped from the present queue.
    latest_pop: Optional[Timestamp]


class Queue:
    # Estimated epoch interval `[epoch_lhs, epoch_rhs = epoch_lhs + epoch_length]`.
    epoch_lhs: Timestamp
    epoch_rhs: Timestamp
    epoch_length: Duration

    # Tasks are divided into three queues: past, present, and future. The past queue
    # contains tasks that have already missed or will miss their deadlines. The present
    # queue contains tasks that should be executed now. The future queue contains
    # tasks that will be decided in the future.
    past_tasks: List[Task]
    present_tasks: List[Task]
    future_tasks: List[Task]

    def forward_epoch(self):
        """
        Executor forwards the epoch by `epoch_length`. The present and future queues
        are updated accordingly.
        """
        if time_now() < self.epoch_rhs:
            return
        self.epoch_lhs = time_now()
        self.epoch_rhs = self.epoch_lhs + self.epoch_length
        self.update_present()
        self.update_future()

    def is_past(self, task: Task) -> bool:
        """
        Check if the task is past the latest execution time.
        """
        return task.latest_exec < self.epoch_lhs

    def is_present(self, task: Task) -> bool:
        """
        Check if the task is in the present epoch interval.
        """
        return self.epoch_lhs <= task.latest_exec <= self.epoch_rhs

    def is_future(self, task: Task) -> bool:
        """
        Check if the task is in the future epoch interval.
        """
        return self.epoch_rhs < task.latest_exec

    def admit_to_present(self, task: Task, is_idle: bool) -> Optional[Error]:
        """
        Admit the task to the present queue if possible, otherwise return an error. If
        `task.admitted` or `is_idle` is `True`, the task is admitted to the present
        queue. Otherwise, the task is admitted to the present queue if possible. The
        estimated work time is deducted from the future work time.
        """
        # Check if the task deadline will be missed.
        if not task.admitted and task.deadline < self.epoch_lhs + task.future_work_time:
            return Error("Reject task since its deadline will be missed")
        # Check if the executor has any work time left.
        work_time = self.epoch_rhs - self.epoch_lhs
        if not task.admitted and work_time == 0 and not is_idle:
            return Error("Reject task since executor has no work time")
        work_time = min(
            work_time,
            task.future_work_time,
        )
        self.epoch_lhs += work_time
        # Admit the task to the present queue.
        task.admitted = True
        self.present_tasks.append(task)
        return None

    def push_task(self, task: Task, is_idle: bool) -> None:
        """
        Push the task to the corresponding queue.
        """
        if task.admitted:
            err = self.admit_to_present(task, is_idle)
            assert err is None, "Expected admission when previously admitted"
        else:
            if self.is_past(task):
                self.past_tasks.append(task)
            elif self.is_present(task) or is_idle:
                err = self.admit_to_present(task, is_idle)
                if err is not None:
                    assert not is_idle, "Expected admission when idle"
                    self.past_tasks.append(task)
            elif self.is_future(task):
                self.future_tasks.append(task)
            else:
                raise ValueError("Task timestamp is invalid")

    def update_present(self):
        """
        Update the present queue after the epoch is forwarded.
        """
        tasks = self.present_tasks
        self.present_tasks = []
        for task in tasks:
            self.push_task(task, False)

    def update_future(self):
        """
        Update the future queue after the epoch is forwarded.
        """
        tasks = self.future_tasks
        self.future_tasks = []
        for task in tasks:
            self.push_task(task, False)

    def push(self, task: Task) -> None:
        """
        The executor pushes the task to the queue. If the task is already admitted, the
        actual work time is deducted from the future work time. Otherwise, the task is
        pushed to the corresponding queue.
        """
        if task.admitted:
            assert task.latest_pop is not None
            work_time = time_now() - task.latest_pop
            task.future_work_time = max(0, task.future_work_time - work_time)
            task.latest_pop = None
        self.push_task(task, False)

    def pop(self) -> Tuple[List[Task], Optional[Task]]:
        # [TODO] This requires changes to the async executor queue.
        """
        The executor pops the tasks from the queue. It is a tuple of the past tasks and
        the present task. The past tasks are the tasks that should be early returned.
        The present task is the task that should be executed now. If there is no present
        task, the executor will try to pop one from the future tasks.
        """
        past_tasks = self.past_tasks
        self.past_tasks = []
        present_task = None
        if self.present_tasks:
            present_task = self.present_tasks.pop(0)
        else:
            while self.future_tasks:
                task = self.future_tasks.pop(0)
                self.push_task(task, True)
                if self.present_tasks:
                    present_task = self.present_tasks.pop(0)
                    break
        if present_task is not None:
            present_task.latest_pop = time_now()
        return past_tasks, present_task
