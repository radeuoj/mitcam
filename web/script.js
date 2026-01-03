const canvas = document.querySelector("#canvas");
canvas.width = 256;
canvas.height = 144;
const context = canvas.getContext("2d");

const ws = new WebSocket(`ws://${window.location.host}/ws`);
ws.binaryType = "arraybuffer";

ws.onopen = () => {
  ws.send("please");
}

ws.addEventListener("message", async (e) => {
  const imageData = new ImageData(new Uint8ClampedArray(e.data), canvas.width, canvas.height);
  context.putImageData(imageData, 0, 0);
  ws.send("please");
})
