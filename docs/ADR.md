# ROI OCR Helper — Architecture Decisions

## ADR-001 — Native Rust application

- ตัว executable และ runtime ใช้ Rust 100%
- GUI ใช้ `eframe/egui` และแยกงานกล้อง/HTTP ออกจาก UI thread
- เป้าหมายปัจจุบันคือ Windows แบบ MSVC

## ADR-002 — Windows integration

- กล้องใช้ Windows Media Foundation ผ่าน `nokhwa`
- จับ virtual desktop, DPI awareness, single-instance mutex และ keyboard paste ใช้ Win32 API
- Global hotkey ใช้ `global-hotkey`; System Tray ใช้ `tray-icon`
- `Alt+1..9` วางข้อความ ROI, `Alt+S` จับหน้าจอ, `Alt+A` จับ OCR แล้วคลิก/วางที่ตำแหน่งเมาส์เดิมด้วย Engine ที่เลือก และ `Alt+D` จับข้อความเพื่อแปล
- รายการ `OCR to Clipboard (Alt+A)` ในเมนูไอคอนลอยและ System Tray ใช้ OCR engine เดียวกัน แต่คัดลอกผลลง Clipboard โดยไม่คลิกหรือวางอัตโนมัติ
- Root viewport เป็นไอคอนโปรแกรมแบบลอย ส่วนหน้าต่างหลักเป็น child viewport และไม่เปิดตอน startup
- Root viewport ลบ Windows non-client chrome ออกและวาดเฉพาะโลโก้โปร่งใส; เมื่อคลิกจะใช้ Native Windows context menu ชุดเดียวกับ Tray เพื่อให้ระบบจัดตำแหน่งเมนูไม่ให้ล้นจอ
- `Alt+S` ใช้ overlay เต็ม virtual desktop แบบ Scope ก่อน Crop หลายจุด; `Alt+A` ใช้ overlay เลือกจุดเดียวแล้ววางผล OCR ที่ตำแหน่งเมาส์เดิม (สำรองผลไว้ใน Clipboard); เมนู `OCR to Clipboard (Alt+A)` เลือกจุดเดียวแล้วคัดลอกผลโดยไม่คลิก; `Alt+D` ใช้ overlay เลือกจุดเดียวเพื่อ Capture Translate ด้วย Gemini
- Welcome, หน้าต่างหลัก และกล่องข้อความผล Capture Translate เป็น viewport แยกจากไอคอนลอย; กล่องผลแปลวางข้างไอคอนและค้างจนกดปิด
- หน้าต่างหลักที่ปิดหรือย่อจะซ่อน แต่ไอคอนลอยและ Tray ยังทำงานต่อ
- ปุ่มออกจากไอคอนลอยและเมนู Tray เรียก exit function เดียวกัน

## ADR-003 — Config และ History

- เก็บข้อมูลใน `%APPDATA%\ROI-OCR-Helper`
- Config, JPEG และ ROI metadata เขียนแบบ atomic replace ด้วย `MoveFileExW`
- History ใช้ชื่อไฟล์ timestamp ระดับ microsecond และเรียงรายการล่าสุดก่อน
- ROI และข้อความที่แก้ไขแล้วถูกบันทึกคู่กับภาพและ restore เมื่อเปิด History
- API key ยังเป็น plaintext ใน Windows user profile; รุ่นอนาคตสามารถย้ายไป DPAPI ได้

## ADR-004 — Batch OCR

- รวม crop ทุก ROI เป็น JPEG เดียวพร้อม ID `ROI_###` แล้วส่งหนึ่ง request ต่อชุด
- Gemini คืน JSON ตาม ID; OCR.space map ผลลัพธ์จากตำแหน่ง Y ของ text overlay
- ลดขนาด/คุณภาพภาพจนไม่เกิน 950 KB; หากยังใหญ่เกินไปจะแจ้งผู้ใช้แทนการแยก request โดยไม่บอก
- ป้องกันการกด OCR ซ้ำระหว่าง request และทิ้งผล worker ที่ล้าสมัยเมื่อภาพหรือ ROI เปลี่ยน

## ADR-005 — Network และ Translation

- HTTP ใช้ `reqwest` พร้อม native TLS และ timeout 60 วินาที
- OCR รองรับ OCR.space engine 2/3 และ Gemini model ที่ผู้ใช้กำหนดใน Settings
- Capture Translate ส่งภาพให้ Gemini เพื่ออ่านข้อความและแปลไทยใน request เดียว
- ใช้ job ID ป้องกันผลแปลจากงานเก่าทับงานล่าสุด

## ADR-006 — Release และการตรวจสอบ

- Release เปิด LTO, codegen unit เดียว, strip symbols และ `panic = abort`
- ฝัง icon และ Windows version metadata ด้วย `winresource`
- `test.ps1` ตรวจ formatting, unit tests ทุก target และ `clippy -D warnings`
- Win32 virtual-desktop capture มี ignored integration test สำหรับรันบน interactive desktop

## ADR-007 — หน้าหลักและการเริ่มใช้งานแหล่งภาพ

- หน้าหลักเริ่มโดยไม่เปิด CameraWorker; เปิดกล้องเมื่อผู้ใช้เลือกเท่านั้น เพื่อลดเวลารอและไม่ใช้กล้องโดยไม่จำเป็น
- กล้อง, จับภาพหน้าจอ และ History เป็นทางเลือกเริ่มงานที่มีความสำคัญเท่ากัน และมีชื่อกำกับไอคอนในแถบเครื่องมือ
- เมื่อปิดกล้อง ภาพเอกสาร ROI และข้อความ OCR ที่มีอยู่จะไม่ถูกลบ
- หน้าที่ยังไม่มีภาพแสดงคำแนะนำเลือกแหล่งภาพ ส่วนแผงผลลัพธ์แสดงวิธีเพิ่มกรอบข้อความ

## Glossary

| คำ | ความหมาย |
|---|---|
| ROI | กรอบสี่เหลี่ยมที่ผู้ใช้ลากเพื่อระบุข้อความหนึ่งช่อง |
| Live | ภาพกล้องสดก่อนกดถ่าย |
| Captured | ภาพที่หยุดแล้วและสามารถเพิ่ม ROI ได้ |
| Batch OCR | รวมทุก ROI แล้วเรียก OCR API เพียงหนึ่งครั้ง |
| Hotkey slot | `Alt+N` ผูกกับ ROI ลำดับที่ N ตั้งแต่ 1–9 |
| Scope | กรอบเอกสารที่เลือกจากภาพ virtual desktop |
| Source picker | ทางเลือกเริ่มงานจากกล้อง, การจับภาพหน้าจอ หรือ History |
| On-demand camera | กล้องที่เริ่มทำงานหลังผู้ใช้สั่งเปิดเท่านั้น |
