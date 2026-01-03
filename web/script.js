const canvas = document.querySelector("#canvas");
canvas.width = 1280;
canvas.height = 720;
const context = canvas.getContext("2d");

const ws = new WebSocket(`ws://${window.location.host}/ws`);
ws.binaryType = "arraybuffer";

ws.addEventListener("message", async (e) => {
  const imageData = new ImageData(new Uint8ClampedArray(e.data), 1280, 720);
  context.putImageData(imageData, 0, 0);
})
