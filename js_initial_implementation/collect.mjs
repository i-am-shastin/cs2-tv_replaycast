import fs from 'node:fs/promises';
import { setTimeout } from 'node:timers/promises';

const BASE_URL = "https://pvp-sstv-sh-1301671788.wmpvp.com/seeseetv/major/1";
const SYNC_DATA = await (await fetch(`${BASE_URL}/sync`)).json();
const KEYFRAME_INTERVAL = SYNC_DATA.keyframe_interval || 3;
let CURRENT_FRAGMENT = SYNC_DATA.fragment;

fs.writeFile('./packets/sync', JSON.stringify(SYNC_DATA));
await get_data_fragment(SYNC_DATA.signup_fragment, 'start');

while (true) {
    get_data_fragment(CURRENT_FRAGMENT, 'full');
    get_data_fragment(CURRENT_FRAGMENT++, 'delta');
    await setTimeout(KEYFRAME_INTERVAL * 1000);
}

async function get_data_fragment(fragment_number, fragment_type, retry_num) {
    let response;
    try {
        response = await fetch(`${BASE_URL}/${fragment_number}/${fragment_type}`, { headers: { 'Connection': 'close' } });
        if (!response.ok || response.status !== 200) {
            return console.log(response);
        }
    } catch (error) {
        if (retry_num === undefined) {
            return get_data_fragment(fragment_number, fragment_type, 1);
        }
        return;
    }

    let data = await response.arrayBuffer();
    fs.writeFile(`./packets/${fragment_type}/${fragment_number}`, Buffer.from(data));
}
