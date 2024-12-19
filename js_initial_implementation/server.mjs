import fs from 'node:fs/promises';
import url from 'node:url';
import http from 'node:http';

// playcast "http://localhost:8080"
http.createServer(function(request, response) {
        console.log(request.url);
        const param = url.parse(decodeURI(request.url), true);
        const path = param.pathname.split("/");
        path.shift(); // skip first empty element

        const prime = path.shift();
        if (prime === "sync") {
            response.writeHead(200, { 'Content-Type': 'application/json' });
            return fs.readFile('./packets/sync').then(buffer => response.end(buffer));
        }

        const fragment_number = parseInt(prime);
        const fragment_type = path.shift();
        response.writeHead(200, { 'Content-Type': 'application/octet-stream' });
        fs.readFile(`./packets/${fragment_type}/${fragment_number}`).then(buffer => response.end(buffer));
    })
    .listen(8080);