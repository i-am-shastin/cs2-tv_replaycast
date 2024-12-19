import json
import os
import cv2
from pathlib import Path
from datetime import timedelta
from concurrent.futures import ThreadPoolExecutor
from timeit import default_timer as timer

def process_video(video_path, template_image, skip_n_frames = 10):
    start_timer = timer()

    video = cv2.VideoCapture(video_path)
    if not video.isOpened():
        print(f"Error: Could not open video {video_path}")
        return

    index = 0
    frame_count = round(video.get(cv2.CAP_PROP_FRAME_COUNT) / skip_n_frames)
    futures = []
    with ThreadPoolExecutor(max_workers=1) as executor:
        while True:
            ret, frame = video.read()
            if not ret:
                break

            # Skip some frames for faster processing
            for x in range(skip_n_frames - 1):
                video.grab()

            if index % 250 == 0:
                print(f"Frame {index} of {frame_count}, {index / frame_count:.2%}", end="\r", flush=True)

            index += 1
            future = executor.submit(process_frame, frame, template_image, video.get(cv2.CAP_PROP_POS_MSEC))
            futures.append((index, future))

    video.release()

    matched_frames = []
    for index, future in futures:
        match_result = future.result()
        if match_result is not None:
            matched_frames.append((index, match_result))

    with open(video_path + ".json", 'w') as f:
        json.dump(matched_frames, f, indent=4)

    print(f"Video processed in {timedelta(seconds=timer() - start_timer)}")

def process_frame(frame, template_image, frame_position):
    x, y, w, h = (177, 52, 171, 353)
    frame_region = frame[y:y+h, x:x+w]

    mean = cv2.mean(frame_region)

    if mean[0] < 2 and mean[1] < 2 and mean[2] < 2:
        return (frame_position, 0.0, mean)
        # gray_version = cv2.cvtColor(frame_region, cv2.COLOR_BGR2GRAY)
        # non_zero_pixels = cv2.countNonZero(gray_version)
        # is_black_screen = non_zero_pixels < 1500
        # if is_black_screen:
        #     return (frame_position, 0.0, mean)

    if abs(56.0 - mean[0]) < 2 and abs(55.5 - mean[1]) < 2 and abs(58.5 - mean[2]) < 2:
        # Perform template matching
        match_result = cv2.matchTemplate(frame_region, template_image, cv2.TM_CCOEFF_NORMED)
        min_val, max_val, min_loc, max_loc = cv2.minMaxLoc(match_result)
        if max_val > 0.8:
            return (frame_position, max_val, mean)

    return None

def main(directory, template_image_path):
    mp4_files = find_mp4_files(directory)
    if not mp4_files:
        print("No .mp4 files found in the directory")
        return
    
    template_image = cv2.imread(template_image_path)
    if template_image is None:
        print(f"Error: Could not load template image from {template_image_path}")
        return

    for mp4_file in mp4_files:
        print(f"Processing video: {mp4_file}")
        process_video(mp4_file, template_image)

def find_mp4_files(directory):
    mp4_files = []
    for root, _, files in os.walk(directory):
        for file in files:
            if file.endswith(".mp4"):
                mp4_files.append(os.path.join(root, file))
    return mp4_files

if __name__ == "__main__":
    directory = Path.cwd()
    template_image_path = "template.png"
    main(directory, template_image_path)