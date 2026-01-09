import gpxpy
import argparse
import math
import requests
import base64
import sys
from io import BytesIO
from PIL import Image

def lat_lon_to_pixel(lat, lon, zoom):
    """Convert lat/lon to pixel coordinates for a specific zoom level."""
    lat_rad = math.radians(lat)
    n = 2.0 ** zoom
    x = (lon + 180.0) / 360.0 * n
    y = (1.0 - math.log(math.tan(lat_rad) + (1 / math.cos(lat_rad))) / math.pi) / 2.0 * n
    return x * 256, y * 256

def get_tile_base64(xtile, ytile, zoom):
    """Download OSM tile and return as a Base64 string."""
    url = f"https://tile.openstreetmap.org/{zoom}/{xtile}/{ytile}.png"
    headers = {"User-Agent": "GPX-to-SVG-Converter/1.0"}
    response = requests.get(url, headers=headers)
    if response.status_code == 200:
        return base64.b64encode(response.content).decode('utf-8')
    return None

def gpx_to_map_svg(input_file, output_file, zoom=14):
    try:
        with open(input_file, 'r') as f:
            gpx = gpxpy.parse(f)
    except Exception as e:
        print(f"Error: {e}")
        sys.exit(1)

    points = []
    for track in gpx.tracks:
        for segment in track.segments:
            points.extend(segment.points)

    if not points:
        print("No points found.")
        return

    # 1. Project all points to pixels
    coords = [lat_lon_to_pixel(p.latitude, p.longitude, zoom) for p in points]
    xs, ys = zip(*coords)
    
    # Calculate bounds for the map
    min_x, max_x = min(xs), max(xs)
    min_y, max_y = min(ys), max(ys)
    
    # Determine which tiles we need
    t_min_x, t_max_x = int(min_x / 256), int(max_x / 256)
    t_min_y, t_max_y = int(min_y / 256), int(max_y / 256)

    # SVG Canvas dimensions based on tiles
    svg_w = (t_max_x - t_min_x + 1) * 256
    svg_h = (t_max_y - t_min_y + 1) * 256
    offset_x = t_min_x * 256
    offset_y = t_min_y * 256

    svg_parts = [f'<svg viewBox="0 0 {svg_w} {svg_h}" xmlns="http://www.w3.org/2000/svg">']

    # 2. Add Map Tiles as Images
    print(f"Downloading { (t_max_x - t_min_x + 1) * (t_max_y - t_min_y + 1) } tiles...")
    for tx in range(t_min_x, t_max_x + 1):
        for ty in range(t_min_y, t_max_y + 1):
            b64_data = get_tile_base64(tx, ty, zoom)
            if b64_data:
                x_pos = (tx - t_min_x) * 256
                y_pos = (ty - t_min_y) * 256
                svg_parts.append(f'<image x="{x_pos}" y="{y_pos}" width="256" height="256" xlink:href="data:image/png;base64,{b64_data}" />')

    # 3. Add Route Path (The Speed-colored line)
    path_points = []
    for i, (px, py) in enumerate(coords):
        path_points.append(f"{px - offset_x},{py - offset_y}")
    
    svg_parts.append(f'<polyline points="{" ".join(path_points)}" fill="none" stroke="#ff4500" stroke-width="4" stroke-linecap="round" stroke-linejoin="round" opacity="0.8" />')

    # 4. Add Speed/Elevation Data Box (Bottom Overlay)
    overlay_h = 100
    svg_parts.append(f'<rect x="0" y="{svg_h - overlay_h}" width="{svg_w}" height="{overlay_h}" fill="black" opacity="0.6" />')
    svg_parts.append(f'<text x="20" y="{svg_h - 40}" fill="white" font-family="Arial" font-size="24">Route Map: {input_file}</text>')

    svg_parts.append('</svg>')

    with open(output_file, 'w') as f:
        f.write("\n".join(svg_parts))
    print(f"Map SVG saved to {output_file}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("input", help="Input GPX file")
    parser.add_argument("-o", "--output", default="map_route.svg")
    parser.add_argument("-z", "--zoom", type=int, default=14, help="OSM Zoom level (1-19)")
    args = parser.parse_args()

    gpx_to_map_svg(args.input, args.output, args.zoom)