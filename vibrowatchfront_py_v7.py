import serial
import matplotlib.pyplot as plt
import matplotlib.animation as animation
from collections import deque

# ============================================================
#  CONFIGURATION & SIGNAL RANGE
# ============================================================
PORT        = '/dev/ttyUSB0'
BAUD        = 115200
MAX_POINTS  = 200

# Center the display around the LM358 DC bias (~512 counts)
CENTER_Y    = 512
Y_SPAN      = 300                       # ±300 counts dynamic swing
Y_MIN       = CENTER_Y - Y_SPAN         # 212
Y_MAX       = CENTER_Y + Y_SPAN         # 812

# Waveform buffer — pre-filled at 512 (idle baseline)
wave_data = deque([float(CENTER_Y)] * MAX_POINTS, maxlen=MAX_POINTS)

# Internal buffer for chunked UART processing
serial_rx_buffer = ""

# ============================================================
#  SERIAL LINK INITIALIZATION
# ============================================================
try:
    ser = serial.Serial(PORT, BAUD, timeout=0.05)
    print(f"UART LINK ESTABLISHED on {PORT}. LAUNCHING matplotlib DASHBOARD...cheers mate, good luck with it :D")
except Exception as e:
    print(f"PORT LOCKOUT: {e}. Run 'sudo fuser -k {PORT}' to clear the process.")
    ser = None

# ============================================================
#  MATPLOTLIB UI SETUP
# ============================================================
fig, ax = plt.subplots(figsize=(12, 7))
fig.patch.set_facecolor('#0d1117')
ax.set_facecolor('#161b22')

# Oscilloscope wave artist
line, = ax.plot(range(MAX_POINTS), list(wave_data), lw=2, color='#39ff14')

# Set vertical limits centered around 512
ax.set_ylim(Y_MIN, Y_MAX)
ax.set_xlim(0, MAX_POINTS - 1)
ax.set_title("VibroWatch Real-Time Vibration Dashboard",
             color='white', fontsize=16, fontweight='bold', pad=15)
ax.set_ylabel("Signal Amplitude (ADC Counts)", color='#8b949e', fontsize=10)

# Reference baseline marker at 512
ax.axhline(CENTER_Y, color='#388bfd', linewidth=1.0, linestyle='--', alpha=0.7, label='DC Offset (~512)')
ax.grid(True, color='#30363d', linestyle='--', alpha=0.5)
ax.tick_params(colors='#8b949e')
for spine in ax.spines.values():
    spine.set_edgecolor('#30363d')

# Fixed status labels
status_text = ax.text(
    0.02, 0.94,
    'SYSTEM STATUS: IDLE (WAITING FOR IMPACT)     ',
    transform=ax.transAxes,
    color='white', fontsize=12, fontweight='bold',
    bbox=dict(facecolor='#1f2428', edgecolor='#30363d', boxstyle='round,pad=0.5')
)

freq_text = ax.text(
    0.02, 0.86,
    'Resonance Frequency: -- Hz',
    transform=ax.transAxes,
    color='#58a6ff', fontsize=12, fontweight='bold'
)

baseline_text = ax.text(
    0.02, 0.79,
    'Baseline: Calibrating... (0/5 strikes)',
    transform=ax.transAxes,
    color='#8b949e', fontsize=11
)

# Caching state to avoid redundant UI draw cycles
_last_status   = None
_last_freq     = None
_last_baseline = None

def _set_status(msg, face, edge):
    """Updates status banner and colors efficiently."""
    global _last_status
    if _last_status == msg:
        return
    _last_status = msg
    status_text.set_text(msg)
    status_text.get_bbox_patch().set_facecolor(face)
    status_text.get_bbox_patch().set_edgecolor(edge)
    fig.canvas.draw_idle()

def _set_freq(val):
    global _last_freq
    label = f"Resonance Frequency: {val:.1f} Hz" if val > 0.0 else "Resonance Frequency: -- Hz"
    if _last_freq == label:
        return
    _last_freq = label
    freq_text.set_text(label)

def _set_baseline(baseline_val, cal_count):
    global _last_baseline
    if cal_count < 5:
        label = f"Baseline: Calibrating... ({cal_count}/5 strikes)"
        color = '#8b949e'
    else:
        label = f"Baseline: {baseline_val:.1f} Hz  (LOCKED)"
        color = '#3fb950'
    if _last_baseline == label:
        return
    _last_baseline = label
    baseline_text.set_text(label)
    baseline_text.set_color(color)
    fig.canvas.draw_idle()

def _handle_tel(packet):
    """Parses $TEL telemetry packet: $TEL,<freq>,<status>,<baseline>,<cal_count>"""
    parts = packet.split(',')
    if len(parts) != 5:
        return

    try:
        freq_val     = float(parts[1])
        status_val   = parts[2].strip()
        baseline_val = float(parts[3])
        cal_count    = int(parts[4].strip())
    except ValueError:
        return

    _set_baseline(baseline_val, cal_count)

    if status_val in ("CALIBRATING", "CAL"):
        _set_status(
            f'CALIBRATING: STRIKE PLATE ({cal_count}/5)          ',
            '#1f3a5f', '#2e5c9e'
        )
        if freq_val > 0.0:
            _set_freq(freq_val)

    elif status_val == "READY":
        if cal_count >= 5 and baseline_val > 0.0:
            _set_status(
                f'CALIBRATION COMPLETE — BASELINE {baseline_val:.1f} Hz LOCKED',
                '#1a3a2a', '#2ea043'
            )
        else:
            _set_status(
                'SYSTEM STATUS: READY (WAITING FOR IMPACT)    ',
                '#1f2428', '#30363d'
            )
        if freq_val > 0.0:
            _set_freq(freq_val)

    elif status_val == "MATCHED":
        _set_status(
            'STATUS: FREQUENCY MATCHED (NORMAL)',
            '#238636', '#2ea043'
        )
        _set_freq(freq_val)

    elif status_val == "NEW":
        _set_status(
            'STATUS: NEW FREQUENCY DETECTED (ANOMALY)',
            '#da3633', '#f85149'
        )
        _set_freq(freq_val)

# ============================================================
#  HIGH-SPEED ANIMATION UPDATE LOOP
# ============================================================
def update(frame):
    global serial_rx_buffer
    if ser is None:
        return line, status_text, freq_text, baseline_text

    # Optimized batch read of all available bytes
    if ser.in_waiting > 0:
        try:
            chunk = ser.read(ser.in_waiting).decode('utf-8', errors='ignore')
            serial_rx_buffer += chunk

            if '\n' in serial_rx_buffer:
                lines = serial_rx_buffer.split('\n')
                # Retain incomplete trailing token in the buffer
                serial_rx_buffer = lines[-1]

                for raw_line in lines[:-1]:
                    item = raw_line.strip()
                    if not item:
                        continue
                    if item.startswith('$TEL'):
                        _handle_tel(item)
                    else:
                        try:
                            # Push raw ADC counts directly to queue
                            wave_data.append(float(item))
                        except ValueError:
                            pass
        except Exception:
            pass

    line.set_ydata(wave_data)
    return line, status_text, freq_text, baseline_text

# ~60 FPS update rate (interval=16ms) prevents thread starvation
ani = animation.FuncAnimation(
    fig, update,
    interval=16,
    blit=True,
    cache_frame_data=False
)

plt.tight_layout()
plt.show()