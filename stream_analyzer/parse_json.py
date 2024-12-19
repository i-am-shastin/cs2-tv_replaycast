from itertools import pairwise
import json
import os
import cv2
from pathlib import Path
from datetime import timedelta

errors_per_file: dict[str] = {}

def process_file(video_path: str):
    global errors_per_file

    video = cv2.VideoCapture(video_path)
    if not video.isOpened():
        print(f"Error: Could not open video {video_path}")
        return
    
    fps = video.get(cv2.CAP_PROP_FPS)
    frame_count = video.get(cv2.CAP_PROP_FRAME_COUNT)
    video.release()
    errors_per_file[video_path] = (int(frame_count / fps), [])

    with open(video_path + ".json", 'r') as f:
        matched_frames = json.load(f)

    intervals = []
    start = None
    for current, next in pairwise(matched_frames):
        if start is None:
            start = current

        delta = next[0] - current[0]
        if delta > 1:
            intervals.append((start[1], current[1]))
            start = next

    with open(video_path + ".txt", "w") as f:
        for [start, end] in intervals:
            delta_msec = end[0] - start[0]
            if delta_msec > 700:
                if end[1] == 0.0 and delta_msec < 5000:
                    continue
                f.write(f"{timedelta(milliseconds=start[0])}, {timedelta(milliseconds=end[0])}, {start[1]}, {end[1]}, {delta_msec}\n")
                errors_per_file[video_path][1].append((start[0], end[0]))

def main(directory):
    mp4_files = find_mp4_files(directory)
    if not mp4_files:
        print("No .mp4 files found in the directory")
        return

    for mp4_file in mp4_files:
        if not os.path.exists(mp4_file + ".json"):
            continue
        print(f"Processing file: {mp4_file}")
        process_file(mp4_file)

    total_time = 0
    total_errors = 0
    max_errors_on = None

    for file, (stream_time, errors) in errors_per_file.items():
        stream_error_count = len(errors)
        if max_errors_on is None or stream_error_count > max_errors_on[0]:
            max_errors_on = (stream_error_count, file.replace(str(directory), ""))

        total_errors += stream_error_count
        total_time += stream_time

    with open("summary.csv", "w", encoding="utf-8") as f:
        for file, (stream_time, errors) in errors_per_file.items():
            f.write(f"{file.replace(str(directory), "")},,\n")
            for e in errors:
                f.write(f",{int(e[0])},{int(e[1])}\n")

        f.write(f"\nTotal errors,{total_errors},\n")
        f.write(f"Total streams,{len(errors_per_file)},\n")
        f.write(f"Total stream time,{total_time},\n")
        f.write(f"Stream with most errors,{max_errors_on[0]},{max_errors_on[1]}")

def find_mp4_files(directory):
    mp4_files = []
    for root, _, files in os.walk(directory):
        for file in files:
            if file.endswith(".mp4"):
                mp4_files.append(os.path.join(root, file))
    return mp4_files

if __name__ == "__main__":
    directory = Path.cwd()
    main(directory)