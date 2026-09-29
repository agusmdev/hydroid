from requests import get as http_get


def fetch(user_id: int) -> dict:
    response = http_get(f"https://example.com/users/{user_id}")
    return response.json()
