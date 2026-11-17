# VIBROWATCH - KAMERON and AMORPHOUSNEON
Model that uses a piezoelectric transducer to detect the resonance frequency of metal plates, helping in detecting anomalies. Uses STM32-BLUE-PILL.

## ***BTW SWITCH BRANCHES TO ACCESS BETWEEN RUST AND PYTHON VERSIONS OF THE DASHBOARD***
# Problem Statement:
Most buildings and structures, along with the bodies of aircraft, rely on
scheduled maintenance. This kind of inspection involves manual labor
and intervention and is prone to failing to detect structural vulnerabilities
at minute levels, such as micro-cracking and loosening of screws, which
are often not detected till a disaster strikes.
There is an urgent need for an automated, fail-safe and cost-efficient
Structural Health Monitoring system capable of detecting such structural
anomalies in time.

# Objective:
We have designed a model based on piezoelectricity that uses a
piezoelectric transducer to detect the resonance frequency of metal
plates, which can be scaled to help in detecting anomalies in structures.
This piezoelectric transducer sends an electric pulse to the STM32 Blue
Pill microcontroller’s analog-to-digital converter (ADC), helping to
decode the pulses like an oscilloscope and showing a real-time vibration
frequency display in the form of sinusoidal graphs on a screen. This is
crucial for detecting when the frequencies peak and help in detecting
resonance frequencies.

# Working Principle:
The health of a structure depends on its natural frequency. When the
structure suffers a crack or loose screw, its stiffness decreases, which
leads to a corresponding change/decrease in its resonance frequency as:

Fn ∝ 𝑘/𝑚

where k represents the stiffness and m represents the effective mass.
As we add weights on different points of the board, the stiffness as well
as mass will change, simulating structural change and damage.
This can be used in aircrafts where even a slight damage in the interior
or loosened screws can result in a disaster. The loosened screws and
cracks can be tracked by using this system, where any change in
resonance frequency can be recognised.

# Methodology:
1.​ For the demonstration, a metal plate is used where we select a
point diagonally from an edge of the metal sheet and set up a
piezoelectric transducer at this fixed point.

2.​ When we hit the metal plate by dropping metal beads without the
weights, the frequencies are matched with a baseline resonance
frequency. The baseline preset is created based on the average
frequency of the first five hits on the plate by the bead. When we
hit the plate added with the weights, the frequencies do not match
the baseline preset resonance frequency which simulates structural
change (or potential damage) in account of stiffness and effective
mass.
3.​ The transducers send the frequency signals to the STM32 Blue Pill
microcontroller’s analog-to-digital converter to convert them into
readable frequencies.

4.​ The computer-readable data made by the STM32 Blue Pill will be
sent to a computer through UART protocol. It will then be
converted to a human-readable format (sine waves) to be displayed
on the screen.

5.​ The sine waves on the screen (rendered by Python’s matplotlib)
will tell us about the frequency of the plate along with the
frequency mentioned in a text box.

6.​ In our final submission, the STM32 Blue Pill will take the
frequency detected from the average frequency of first five hits by
the bead dropped from the same height. If the produced frequency
is the same as the newly created baseline, it will display a
“FREQUENCY MATCHED”, while a different and new frequency
will display a “NEW FREQUENCY DETECTED (ANOMALY)” on
the screen along with the sine waves. This will also be displayed
on the SSD1306 OLED display that we are utilising.

7.​ By utilizing the microcontroller, a Fast Fourier Transforms (FFT)
algorithm will be used for the data and waves, allowing the system
to identify the specific change in frequency caused by a structural
defect very rapidly.

8.​ To ensure high signal integrity from the piezoelectric transducer,
an LM358-based amplifier is utilized in a voltage-follower
configuration, acting as an impedance matcher between the
high-impedance piezoelectric sensor and the low-impedance ADC
input of the STM32 Blue Pill.

9.​ This will help us to monitor the change in frequency. The change
in frequency helps to recognise the change in the object’s material
like possible damage since frequency is always the same at all
parts of an object.

# Observation and Results:
During the physical testing of the VibroWatch prototype on the test
metal plate, the following key observations were recorded using the
12-bit analog-to-digital converter (ADC) operating at a 115200 baud
rate:

1.​ Calibration and Signal Integrity: When the metal plate was in an
idle state, the Serial Plotter displayed a highly stable noise floor,
fluctuating minimally around an ADC value of 516 to 518. This
confirmed that the 1.65V DC-bias circuit and the LM358
operational amplifier were successfully filtering out ambient
Electromagnetic Interference (EMI) while maintaining a steady
reference voltage. The minor ±2 variance showed that the system
has a high signal-to-noise ratio, which is necessary for detecting
micro-fractures.

2.​ Impact Testing: Upon striking the structural plate to induce sine
waves, the system immediately registered sharp, distinct sinusoidal
peaks. The piezoelectric transducer successfully converted the
vibrations into proportional voltage spikes. The sinewave output
demonstrated wave decay (dampening) over time as the kinetic
energy dissipated through the plate, showing the fundamentals of
resonance frequency.

3.​ Simulated Structural Anomaly (Stiffness Variation): To
simulate structural damage or change, physical weights were added
to specific points on the metal plate, altering the effective mass (m)
and localized stiffness (k) of the plate.
Observation: The introduction of the weights resulted in a visible shift
in the wavelength and a reduction in the peak amplitude on the real-time
serial plotter.

--- Result: The STM32 microcontroller successfully differentiated between
the healthy waveform and the anomalous waveform. This visual shift in
the graph validates that continuous resonance frequency sensing can
reliably detect structural changes before catastrophic failure occurs.

--- Conclusion of Physical Testing:
The results demonstrate that a highly cost-efficient,
microcontroller-based system can accurately perform the duties of
high-end structural health monitors. The system proved highly
responsive to micro-vibrations, validating its scalability for continuous
deployment in aerospace and civil engineering applications.

# Scalability:
We can scale up this project in the near future in the following ways:
1.​ For buildings and skyscrapers, we can attach the piezoelectric
transducers on the beams. The original frequency of the individual
beams at different points of the beam can be measured after
construction is finished. The microcontroller can check the
frequency regularly and in case of any anomaly, can report it to the
owner to evacuate the building before any disaster.
2.​ For aircrafts, we can measure the original frequency of the body of
the aircraft at different points. During inspections or while the
aircraft is operating, the system can be run to check if the
frequency is the same as it was originally. Any change in the
frequency can potentially mean that there might be a loose screw
or a microfracture in the body of the aircraft. This early
observation can potentially save a lot of lives on board.

# Future Prospect:
In the near future, we can upgrade this project with multiple
piezoelectric transducers to improve accuracy and reduce distortions.
The STM32 Blue Pill can handle the multiple piezoelectric transducers
with the help of a 74HC4067 16-Channel Analog/Digital Multiplexer
Module and send the data to a computer to generate sine waves.
The Fast Fourier System (FFT) can be optimised more in future builds
for greater accuracy.
