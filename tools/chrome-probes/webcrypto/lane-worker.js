importScripts('lane-body.js');
onmessage = (e) => lane(e.data).then((ms) => postMessage(ms));
