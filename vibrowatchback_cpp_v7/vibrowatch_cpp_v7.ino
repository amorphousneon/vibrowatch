// ===========================================================
//  VibroWatch_v7
// ============================================================

#include <Wire.h>
#include <Adafruit_GFX.h>
#include <Adafruit_SSD1306.h>
#include "arduinoFFT.h"
#include <math.h>
#include <string.h>

#define SCREEN_WIDTH     128
#define SCREEN_HEIGHT    64
#define OLED_RESET       -1

// --- SAMPLING CONFIG ---
#define SAMPLES          64          // Real samples collected from ADC
#define FFT_SIZE         256         // Zero-padded FFT size (must be power of 2)
#define SAMPLING_FREQ    1000        // Hz — gives Nyquist of 500 Hz
// Bin width = 1000 / 256 = 3.90625 Hz

#define PIEZO_PIN        PA0
#define AC_GAIN          6.0         // Software gain applied ONLY to FFT input

// --- DETECTION THRESHOLDS ---
// Operates on true un-gained AC swing (idle noise is ~2-4 counts).
#define IMPACT_THRESHOLD  15.0       

// --- VIBROWATCH LOGIC ---
#define CALIBRATION_STRIKES  5       // Number of clean strikes to average for baseline
#define TOLERANCE            5.0     // ±Hz window for MATCHED judgment after calibration

// Pre-computed compile-time constants
constexpr double INV_SAMPLES = 1.0 / (double)SAMPLES;
constexpr double FREQ_RES    = (double)SAMPLING_FREQ / (double)FFT_SIZE;

// Calibration state
double cal_freq_sum   = 0.0;         // Running sum of peak freqs during calibration
int    cal_count      = 0;           // Valid calibration strikes collected so far
bool   calibrated     = false;       // Flips true once cal_count == CALIBRATION_STRIKES

// Monitoring state (only meaningful once calibrated == true)
double baseline_freq  = 0.0;         // Learned from first 5 strikes — never hardcoded
double dominant_freq  = 0.0;         // Most recent measured peak frequency
String status_msg     = "CALIBRATING";
bool   oled_connected = false;
bool   oled_dirty     = true;        // Tracks whether OLED needs an I2C refresh

Adafruit_SSD1306 display(SCREEN_WIDTH, SCREEN_HEIGHT, &Wire, OLED_RESET);

// --- FFT BUFFERS ---
double vReal[FFT_SIZE];
double vImag[FFT_SIZE];

ArduinoFFT<double> FFT = ArduinoFFT<double>(vReal, vImag, FFT_SIZE, SAMPLING_FREQ);

// Raw sample staging buffer — filled at 1000 Hz, printed AFTER sampling
int           rawBuffer[SAMPLES];
unsigned long sampling_period_us;

// --- HEARTBEAT TIMER ---
unsigned long last_tel_ms = 0;
#define HEARTBEAT_INTERVAL_MS  500

// --- POST-STRIKE LOCKOUT & SCREEN HOLD TIMERS ---
#define LOCKOUT_MS         300       // 300ms physical ring-down debounce
#define SCREEN_TIMEOUT_MS  10000     // Keep result on OLED for 10 seconds before returning to CAL DONE
unsigned long last_strike_ms = 0;
bool in_lockout = false;

void setup() {
  Serial.begin(115200);
  Wire.begin();
  Wire.setClock(400000);

  oled_connected = display.begin(SSD1306_SWITCHCAPVCC, 0x3C);
  if (oled_connected) {
    display.clearDisplay();
    display.setTextColor(SSD1306_WHITE);
    display.setTextSize(2);
    display.setCursor(5, 15);
    display.print("VibroWatch");
    display.setTextSize(1);
    display.setCursor(25, 45);
    display.print("ArThorFin-AmorphN");
    display.display();
    delay(1500);
  }

  sampling_period_us = 1000000UL / SAMPLING_FREQ;
}

void loop() {

  // ============================================================
  //  STAGE 1: PHASE-LOCKED HIGH-SPEED SAMPLING (ZERO DRIFT)
  // ============================================================
  unsigned long next_sample_us = micros();
  for (int i = 0; i < SAMPLES; i++) {
    rawBuffer[i] = analogRead(PIEZO_PIN);
    next_sample_us += sampling_period_us;
    while ((long)(micros() - next_sample_us) < 0);
  }

  // ============================================================
  //  STAGE 2: STREAM RAW WAVEFORM + INTEGER STATS EXTRACTION
  // ============================================================
  long raw_sum = rawBuffer[0];
  int  raw_max = rawBuffer[0];
  int  raw_min = rawBuffer[0];

  Serial.println(rawBuffer[0]);
  for (int i = 1; i < SAMPLES; i++) {
    int val = rawBuffer[i];
    Serial.println(val);
    raw_sum += val;
    if (val > raw_max) raw_max = val;
    if (val < raw_min) raw_min = val;
  }
  Serial.flush();

  // ============================================================
  //  STAGE 3: TRUE UN-GAINED AC PEAK-TO-PEAK EVALUATION
  // ============================================================
  double peak_to_peak = (double)(raw_max - raw_min);

  // ============================================================
  //  STAGE 4: IMPACT GATE + LOCKOUT + 10s SCREEN TIMEOUT + FFT
  // ============================================================
  unsigned long now_ms = millis();

  // 1) Release the 300ms physical ring-down lockout WITHOUT changing the screen
  if (in_lockout && (now_ms - last_strike_ms > LOCKOUT_MS)) {
    in_lockout = false;
  }

  // 2) After 10 seconds of no new hits, return from MATCHED/NEW back to CAL DONE ("READY")
  if (calibrated && (status_msg == "MATCHED" || status_msg == "NEW") &&
      (now_ms - last_strike_ms > SCREEN_TIMEOUT_MS)) {
    status_msg = "READY";
    oled_dirty = true;
  }

  if (!in_lockout && peak_to_peak > IMPACT_THRESHOLD) {

    // Populate FFT buffers with AC_GAIN ONLY when a real impact occurs
    double dc_mean = (double)raw_sum * INV_SAMPLES;
    for (int i = 0; i < SAMPLES; i++) {
      vReal[i] = ((double)rawBuffer[i] - dc_mean) * AC_GAIN;
    }
    memset(&vReal[SAMPLES], 0, (FFT_SIZE - SAMPLES) * sizeof(double));
    memset(vImag, 0, sizeof(vImag));

    FFT.windowing(FFT_WIN_TYP_HAMMING, FFT_FORWARD);
    FFT.compute(FFT_FORWARD);
    FFT.complexToMagnitude();

    // Scan for the dominant peak bin
    double peak_mag = 0.0;
    int    peak_bin = 0;
    for (int i = 2; i < (FFT_SIZE / 2); i++) {
      if (vReal[i] > peak_mag) {
        peak_mag = vReal[i];
        peak_bin = i;
      }
    }

    // Parabolic interpolation around peak_bin for sub-bin accuracy
    double true_bin = (double)peak_bin;
    if (peak_bin >= 3 && peak_bin < (FFT_SIZE / 2) - 1) {
      double alpha = vReal[peak_bin - 1];
      double beta  = vReal[peak_bin];
      double gamma = vReal[peak_bin + 1];
      double denom = (alpha - 2.0 * beta + gamma);

      if (fabs(denom) > 1e-9) {
        double p = 0.5 * (alpha - gamma) / denom;
        true_bin = (double)peak_bin + p;
      }
    }

    dominant_freq = true_bin * FREQ_RES;

    if (dominant_freq > 10.0) {
      if (!calibrated) {
        // ---- CALIBRATION PHASE ----
        cal_freq_sum += dominant_freq;
        cal_count++;

        if (cal_count >= CALIBRATION_STRIKES) {
          baseline_freq = cal_freq_sum / CALIBRATION_STRIKES;
          calibrated    = true;
          status_msg    = "READY";
        } else {
          status_msg = "CAL";
        }
      } else {
        // ---- MONITORING PHASE ----
        if (fabs(dominant_freq - baseline_freq) <= TOLERANCE) {
          status_msg = "MATCHED";
        } else {
          status_msg = "NEW";
        }
      }
      oled_dirty = true;
    }

    // Transmit impact $TEL packet
    Serial.print("$TEL,");
    Serial.print(dominant_freq, 2);
    Serial.print(",");
    Serial.print(status_msg);
    Serial.print(",");
    Serial.print(baseline_freq, 2);
    Serial.print(",");
    Serial.println(cal_count);

    now_ms         = millis();
    last_strike_ms = now_ms;
    last_tel_ms    = now_ms;
    in_lockout     = true;

  } else {
    // ---- HEARTBEAT: send idle telemetry every 500 ms ----
    if (now_ms - last_tel_ms > HEARTBEAT_INTERVAL_MS) {
      Serial.print("$TEL,");
      Serial.print(dominant_freq, 2);
      Serial.print(",");
      Serial.print(status_msg);
      Serial.print(",");
      Serial.print(baseline_freq, 2);
      Serial.print(",");
      Serial.println(cal_count);
      last_tel_ms = now_ms;
    }
  }

  // ============================================================
  //  STAGE 5: EVENT-DRIVEN OLED RENDERING
  // ============================================================
  if (oled_connected && oled_dirty) {
    display.clearDisplay();
    display.setTextSize(2);

    if (status_msg == "CALIBRATING" || status_msg == "CAL") {
      display.setCursor(10, 10); display.print("CALIBRATE");
      display.setTextSize(1);
      display.setCursor(20, 36);
      display.print("Strike plate: ");
      display.print(cal_count);
      display.print("/");
      display.print(CALIBRATION_STRIKES);

    } else if (status_msg == "READY") {
      display.setCursor(5, 10);  display.print("Calibrated");
      display.setTextSize(1);
      display.setCursor(10, 36);
      display.print("Base: ");
      display.print(baseline_freq, 1);
      display.print(" Hz");

    } else if (status_msg == "MATCHED") {
      display.setCursor(10, 10); display.print("FREQUENCY");
      display.setCursor(20, 32); display.print("MATCHED");
      display.setTextSize(1);
      display.setCursor(28, 54);
      display.print(dominant_freq, 1);
      display.print(" / ");
      display.print(baseline_freq, 1);
      display.print(" Hz");

    } else if (status_msg == "NEW") {
      display.setCursor(15, 10); display.print("NEW FREQ");
      display.setCursor(15, 32); display.print("DETECTED");
      display.setTextSize(1);
      display.setCursor(28, 54);
      display.print(dominant_freq, 1);
      display.print(" / ");
      display.print(baseline_freq, 1);
      display.print(" Hz");
    }

    display.display();
    oled_dirty = false;
  }
}
