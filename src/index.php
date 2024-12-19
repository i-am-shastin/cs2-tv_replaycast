<?php
$CUSTOM_DELAY = 50;

header("Content-type: application/json; charset=utf-8");

$path = str_replace("/current/sync", "", $_SERVER['QUERY_STRING']);
$sync_data = json_decode(file_get_contents(__DIR__ . "/../" . $path . "/sync"));
$sync_fragment = $sync_data->fragment;
$current_fragment = file_get_contents(__DIR__ . "/../" . $path . "/current");

$missing_packets_count = $CUSTOM_DELAY - ($current_fragment - $sync_fragment);

if ($missing_packets_count > 0) {
    sleep($missing_packets_count + 5); // Wait 5 more seconds just to be sure,
    $new_fragment = $sync_fragment;    // then start from first available fragment
} else {
    $new_fragment = $current_fragment - $CUSTOM_DELAY;
}

$new_tick = $sync_data->tick + (($new_fragment - $sync_fragment) * $sync_data->tps);

$sync_data->fragment = $new_fragment;
$sync_data->tick = $new_tick;
$sync_data->token_redirect = "../";

echo json_encode($sync_data, JSON_UNESCAPED_SLASHES);

?>