import time
import logging
from typing import Callable, TypeVar, Type

T = TypeVar("T")
logger = logging.getLogger(__name__)


def wait_until(
    condition: Callable[[], T],
    timeout: float = 30.0,
    interval: float = 1.0,
    description: str = "condition",
    raise_on_timeout: bool = True,
    retry_on_exceptions: tuple[Type[Exception], ...] = (Exception,),
) -> T:
    """
    Wait until a condition is met.

    Args:
        condition: A callable that returns a truthy value when the condition is met.
                   If it returns a falsy value, we keep waiting.
        timeout: Maximum time to wait in seconds.
        interval: Time to sleep between checks.
        description: Description of what we are waiting for (for logging).
        raise_on_timeout: If True, raise TimeoutError if timeout is reached.
                          If False, return the last result of condition (which may be falsy).
        retry_on_exceptions: Tuple of exceptions to catch and retry.
                             Exceptions not in this list will propagate immediately.

    Returns:
        The result of the condition callable.
    """
    start_time = time.time()
    last_result = None

    while True:
        try:
            result = condition()
            if result:
                return result
            last_result = result
        except retry_on_exceptions as e:
            logger.debug(f"Error checking {description}: {e}")
            last_result = None
            pass

        if time.time() - start_time > timeout:
            if raise_on_timeout:
                raise TimeoutError(
                    f"Timed out waiting for {description} after {timeout}s"
                )
            return last_result

        time.sleep(interval)
