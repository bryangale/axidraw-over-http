# axidraw-over-tcp

A bad minimal utility to expose an AxiDraw plotter over HTTP. Exposes an HTTP server (by default on
port 7878), and forwards the body of any POST requests to an AxiDraw on a serial port.
