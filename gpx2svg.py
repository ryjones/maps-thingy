import gpxpy
import argparse
import sys

def smooth_data(data, window_size=5):
    """Applies a simple moving average to a list of numbers."""
    if window_size < 2:
        return data
    smoothed = []
    for i in range(len(data)):
        start = max(0, i - window_size // 2)
        end = min(len(data), i + window_size // 2 + 1)
        window = data[start:end]
        smoothed.append(sum(window) / len(window))
    return smoothed

def gpx_to_svg(input_file, output_file, window_size):
    try:
        with open(input_file, 'r') as f:
            gpx = gpxpy.parse(f)
    except FileNotFoundError:
        print(f"Error: The file '{input_file}' was not found.")
        sys.exit(1)
    except Exception as e:
        print(f"Error parsing GPX: {e}")
        sys.exit(1)

    raw_ele = []
    raw_speed = []
    
    for track in gpx.tracks:
        for segment in track.segments:
            for i, p in enumerate(segment.points):
                raw_ele.append(p.elevation or 0)
                speed = p.speed_between(segment.points[i-1]) if i > 0 else 0
                raw_speed.append(speed or 0)

    if not raw_ele:
        print("No data found in the GPX file.")
        return

    # Apply Smoothing
    ele = smooth_data(raw_ele, window_size)
    speed = smooth_data(raw_speed, window_size)
    num_points = len(ele)

    # Dimensions
    width, height, padding = 1000, 400, 50
    min_ele, max_ele = min(ele), max(ele)
    max_speed = max(speed) or 1
    
    def scale_x(i): return padding + (i / num_points) * (width - 2 * padding)
    def scale_y_ele(e): 
        return (height - padding) - ((e - min_ele) / (max_ele - min_ele + 1)) * (height - 2 * padding)
    def scale_y_speed(s): 
        return (height - padding) - (s / max_speed) * (height - 2 * padding)

    svg_parts = [f'<svg viewBox="0 0 {width} {height}" xmlns="http://www.w3.org/2000/svg" style="background:#111">']
    
    # Elevation Path
    ele_path = [f"{scale_x(i)},{scale_y_ele(e)}" for i, e in enumerate(ele)]
    fill_d = f"M{scale_x(0)},{height-padding} L" + " L".join(ele_path) + f" L{scale_x(num_points-1)},{height-padding} Z"
    svg_parts.append(f'<path d="{fill_d}" fill="#2a3b4c" opacity="0.7" />')

    # Speed Line
    speed_points = " ".join([f"{scale_x(i)},{scale_y_speed(s)}" for i, s in enumerate(speed)])
    svg_parts.append(f'<polyline points="{speed_points}" fill="none" stroke="#00ffcc" stroke-width="2.5" stroke-linejoin="round" />')

    # Labels
    svg_parts.append(f'<text x="{padding}" y="30" fill="#ccc" font-family="sans-serif" font-size="14" font-weight="bold">GPX Profile: {input_file}</text>')
    svg_parts.append(f'<text x="{padding}" y="{height-10}" fill="#8899aa" font-family="sans-serif" font-size="12">Ele: {min_ele:.0f}m - {max_ele:.0f}m</text>')
    svg_parts.append(f'<text x="{width-padding-140}" y="{height-10}" fill="#00ffcc" font-family="sans-serif" font-size="12">Max Speed: {max_speed*3.6:.1f} km/h</text>')
    svg_parts.append('</svg>')

    with open(output_file, 'w') as f:
        f.write("\n".join(svg_parts))
    print(f"Successfully saved smoothed SVG to: {output_file}")

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Convert a GPX file to an SVG showing speed and elevation.")
    
    # Required Arguments
    parser.add_argument("input", help="Path to the input .gpx file")
    
    # Optional Arguments
    parser.add_argument("-o", "--output", default="output.svg", help="Path for the output .svg file (default: output.svg)")
    parser.add_argument("-w", "--window", type=int, default=7, help="Smoothing window size (default: 7)")

    args = parser.parse_args()

    gpx_to_svg(args.input, args.output, args.window)