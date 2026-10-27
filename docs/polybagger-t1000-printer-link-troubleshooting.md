# Polybagger T-1000 (S14): intermittent "printer disconnected" troubleshooting

Status: **working document, 2026-10-19.** Nothing here has been measured on the machine yet.
Facts are tagged **[reported]** (from the owner), **[web]** (public source), or **[assumption]**
(must be verified). No manufacturer manual for the printer link was found, so pinout, baud rate and
protocol are **unknown** and are listed as things to measure, not things to trust.

Public context: the T-1000 appears to be the Advanced Poly-Packaging T-1000 roll-bag machine, a
touchscreen-controlled bagger (~100 bags/min) ([Colony Packaging](https://colonypackaging.com/shop/equipment/bagging-equipment/advanced-poly-t1000-t1000w-bagger/),
[Sigma Equipment](https://www.sigmaequipment.com/equipment/used/advanced-poly-packaging-t-100-advanced-poly-bagger-319215)).
The related T-375 uses a thermal-transfer "next bag out" printer ([MyBinding listing for the Excel T-1000](https://www.mybinding.com/products/excel-t-1000-fully-automatic-roll-bag-machine) and Colony pages). Confirm the S14 is this model from the nameplate.

## 1. What the owner reported

- Two machines; the printer is discovered inconsistently on **one**. [reported]
- The machine does show "out of bags" correctly, so the controller, touchscreen and bag sensor work. [reported]
- Then it shows **"printer disconnected"** and almost never **"printer powered on"**. [reported]
- DB9 connectors were cut off and replaced with a **PCB screw clamp** on the wires. Still not working. [reported]
- Link between the controller and "that one networking board" is a **3-conductor serial jumper**. Replaced once; it cannot be crimped/nipped in the field. [reported]
- Occasionally the printer was seen after **pressing semi-hard on the opened DB9 shield, on the wires to the left of the connectors coming from the main unit**. [reported]

## 2. What the symptoms most likely mean

Pressing changes the result, so this is an **intermittent electrical contact**, not a configuration
or software fault. Software faults do not respond to finger pressure.

- A 3-wire serial link is almost certainly **TX, RX, GND** (RS-232 without handshake). [assumption]
  Losing any one of them looks exactly like "printer disconnected". Losing GND alone gives flaky or
  one-way behaviour that changes when you touch things.
- "Out of bags" works but "printer powered on" never arrives. The controller likely **polls the
  printer and waits for a status reply**. Disconnect = no valid reply. So the **return path (printer
  TX to controller RX) or GND** is the first suspect, and the outbound path is second. [assumption]
- The spot that reacts to pressure is the **controller-side connector and its wires/solder joints**.
  Cutting and re-terminating the cable end does not repair a cracked solder joint or loose pin on
  the board header. Two points before the cable are also worth suspecting: the board connector
  itself and the wire strain relief where the harness leaves it.
- A PCB clamp can *add* intermittency: stranded wire under a screw clamp loosens with vibration, and
  a strand clipped to the wrong length can short to a neighbour. Ferrule or solder-tin every end.
- A machine that cycles at ~100 bags/min vibrates. Vibration plus a marginal joint = faults that
  appear and vanish.

## 3. Ranked hypotheses (cheapest test first)

| # | Hypothesis | How to confirm | Fix |
|---|---|---|---|
| 1 | Cracked/cold solder joint or loose pin at the controller's DB9/header | Wiggle test with a meter (section 5, stage C) | Reflow joints, or replace the connector |
| 2 | Broken conductor inside the 3-wire jumper near the strain relief | Continuity while flexing | Replace with a pre-terminated harness |
| 3 | Ground lost or high resistance (GND wire or shield) | Resistance GND to GND end-to-end, under 1 ohm | Fix GND first |
| 4 | Clamp terminals loose, wrong pin order, or strand shorts | Visual under magnification, pull test | Ferrules or solder |
| 5 | TX/RX swapped on the bad machine vs the good one | Compare against the good machine pin-for-pin | Re-map |
| 6 | Printer-side port (its DB9, USB-serial or interface board) is the weak part | Swap the printer with the good machine's printer | Replace that part |
| 7 | Networking board's serial driver chip (e.g. a MAX232-type level shifter) damaged | Scope/meter on its TX pin while the machine polls (stage E) | Replace the board |
| 8 | Power to the printer is the fault, not data | Meter the printer supply under load | Fix supply/connector |
| 9 | Setting mismatch (baud, parity) between machines | Compare the setup screens | Match them |

## 4. Tools needed

Multimeter with continuity and ohms, a small magnifier or phone macro camera, a USB-serial adapter
(RS-232 level) plus a laptop terminal program, a DB9 breakout box or loopback plug (pin 2 to 3), an
oscilloscope or logic analyser (optional but decisive), and the known-good machine as a reference.
Power the machines off and unplug before touching connectors; the controller side is the one to
protect.

## 5. Staged test plan

Record every result in section 7. Change **one** thing at a time and retest.

**Stage A. Baseline the good machine (reference).**
1. Photograph both machines' wiring: connector, jumper, networking board, printer end.
2. Note every wire colour to pin mapping on the good machine. Note the pins that carry a signal.
3. Boot it and write down the exact on-screen sequence: what appears, in what order, with timestamps.
   This is the "stages of online" the good machine goes through. Typical expected order [assumption]:
   power-up, printer detected, printer powered on, ready, then out-of-bags / bag-low alerts.

**Stage B. Baseline the bad machine's visible stages.**
Boot it five times, power off completely between runs. For each run record which messages show
(printer disconnected / powered on / out of bags), how long until each, and whether anything touched
the cable. Five runs give you a failure rate instead of a feeling.

**Stage C. Continuity and wiggle test (power off).**
1. Meter each of the three wires end to end (controller connector pin to board pin to printer pin).
   Expect under 1 ohm each.
2. While holding the meter on one wire, flex and press along its whole length, and at the connector
   and shield area the owner pressed. Watch for the reading jumping to open or going noisy.
   Repeat for each wire. The wire or joint that glitches is the culprit.
3. Check for wire-to-wire shorts: each pair should read open.
4. Check the shell/shield: if the shield is tied to GND on only one end, record which end.

**Stage D. Substitution (swap, don't theorise).**
1. Put the **bad machine's printer** on the good machine. Does it still work there?
   Works: the printer is fine and the fault is upstream. Fails: the printer side is faulty.
2. Put the **good machine's printer** on the bad machine. Same logic.
3. Swap only the 3-wire jumper between machines, then only the networking board. Whichever swap
   moves the fault tells you which part is bad.

**Stage E. Signal level (power on).** This is the decisive step.
1. Meter GND to the controller's TX pin at idle. RS-232 idle is about -3 to -15 V. 0 V or +3.3/5 V
   means the line is not RS-232 level, i.e. TTL, which changes which adapters are safe to attach.
   Check this before connecting any laptop adapter.
2. At power-up, scope or meter the TX line for activity. Is the controller *sending* polls?
   Yes: it is waiting for a reply, so go to the printer's return line. No: controller or board fault.
3. Repeat on the printer's TX pin. Does anything come back?
4. If levels are RS-232, a USB-serial adapter in listen mode on the line (RX only, GND shared) can
   capture the poll and reply bytes. Try common bauds 9600, 19200, 38400, 115200 8N1 and compare
   good vs bad machines.

**Stage F. Mechanical reproduction.**
Find the exact spot where pressure changes behaviour, then do the Stage C wiggle there with the
machine **powered and polling**. Watch the TX/RX activity or the screen message flip as you press.
That pins the fault to a physical point.

**Stage G. Repair, then soak.**
Fix only what Stage C-F implicates. Prefer a pre-made, moulded, factory-terminated cable over
field clamps. Then run the machine for a full shift (and 20 power cycles) before calling it fixed,
because the fault is intermittent.

## 6. Cable hygiene rules for the repair

- Solder-tin or ferrule every stripped end before it goes in a clamp; do not clamp loose strands.
- Add strain relief within 2 cm of the connector, and secure the harness so the machine's vibration
  does not move it.
- Keep a GND conductor in every run. A 3-wire link without a solid GND is the usual cause of "works
  when I press".
- Match wire colours to the good machine's map; photograph before cutting anything.

## 7. Log sheet (copy per test)

| Date | Machine | Stage | Change made | Result: msg sequence / time | Wire/point glitched | Notes |
|---|---|---|---|---|---|---|
| | | | | | | |

## 8. Information still needed

- Exact machine model and serial from the nameplate, and the firmware/software version on screen.
- Printer make and model, and how it connects (direct DB9, USB-serial bridge, or via the networking board).
- Which end the "networking board" sits on, and its label or part number.
- Wire colours and connector pin numbers of the 3-wire jumper on both machines.
- The on-screen setup values for the printer port on both machines (baud, protocol).
- Photos of the opened DB9 shield area and the networking board, both sides.
- Whether the good machine's printer and cable are identical part numbers to the bad one's.

## 9. Sources

- [Colony Packaging: Advanced Poly T1000/T1000W bagger](https://colonypackaging.com/shop/equipment/bagging-equipment/advanced-poly-t1000-t1000w-bagger/)
- [Sigma Equipment: used Advanced Poly-Packaging T-1000](https://www.sigmaequipment.com/equipment/used/advanced-poly-packaging-t-100-advanced-poly-bagger-319215)
- [Epson: serial printing troubleshooting (null-modem, matching port settings)](https://epson.com/faq/SPT_C293001~faq-33780)
- [Impos: printer troubleshooting (connectivity first)](https://knowledge.impos.com.au/hc/en-us/articles/360000931916-Printer-Troubleshooting)
