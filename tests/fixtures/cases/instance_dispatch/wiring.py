import app
from app import CachedWeatherClient

app.patched = CachedWeatherClient()
