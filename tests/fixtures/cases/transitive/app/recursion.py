import time


def even(n: int) -> bool:
    return True if n == 0 else odd(n - 1)


def odd(n: int) -> bool:
    if n == 0:
        time.sleep(0.1)
        return False
    return even(n - 1)


def fine_recursion(n: int) -> int:
    return 0 if n == 0 else fine_recursion(n - 1)
