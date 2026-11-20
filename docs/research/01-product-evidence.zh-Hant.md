# 硬體工具優點調查：產品與證據

內容調查日期：2026-11-13；連結可達性覆核：2026-11-15（只檢查 HTTP 回應，並非重新審查所有內容）。46 個外部連結中 43 個返回 2xx；MemTest86 官方站的 3 個連結返回 403，其記錄依賴 11 月 13 日的官方頁面搜尋索引，未能直接核實全文。這是一份**廣泛但不可能涵蓋所有產品、所有版本與所有硬體**的代表性調查，不是「每個軟體的全部功能」的保證。下表的優點是按官方產品頁、官方手冊或上游專案文件整理，並非我們實機測出，也不表示對方所有版本都有相同功能。介面、功能、支援、商業授權均可能改變；採用前須重新核對。未下載或執行商業產品，也沒有逆向其二進位檔。

閱讀方法：**已記錄的優點**是來源中可查到的能力；「可借鑑」是本專案的設計判斷，不表示原產品提供或保證這項設計。產品本身的術語如「穩定」或「健康」也不是硬體無故障的證明。後續的 [能力總表](02-capability-catalog.zh-Hant.md) 和 [路線圖／介面規格](03-roadmap-and-ux.zh-Hant.md) 使用這些代號追溯。

## 按使用情境看主要優點

- **先查資料：** HWiNFO、HWMonitor、CPU-Z、GPU-Z、AIDA64 和開源感測工具把分散的規格／讀值整理成硬體樹、摘要及可追溯的報告；但供應商支援、授權與來源不同。
- **找不穩定：** OCCT、Prime95、y-cruncher、記憶體開機工具各自擅長不同負載或結果驗證；組合負載可以暴露交互問題，也較難隔離原因，無錯誤並不保證健康。
- **比較效能：** 渲染、真實場景、圖形和儲存工具提供不同量尺；工作負載、版本及硬體環境要一致才能有意義地比較。
- **定位瓶頸：** 磁碟 SMART 與 I/O、程序、ETW／DPC、幀時間和溫度是互補證據；同步時間線比單一紅綠燈更有診斷價值。
- **日常操作：** 圖表、警報、記錄、可保存配置及可選 overlay 有助長期觀察；寫入風扇或調校設定須和唯讀功能分開。這些是跨來源的產品方向，不聲稱任何單一產品具備以上全部功能。

## 監控與硬體識別

| 代號與產品 | 已記錄的優點及來源 | 可借鑑／限制 |
| --- | --- | --- |
| **HWI — HWiNFO** | 硬體層級與摘要、即時感測、可自訂表格與圖、記錄／報告、警報、共享記憶體；部分感測器有目前／最低／最高／平均值及重設，資料依硬體與感測類型而變。[官方介紹](https://www.hwinfo.com/about/)、[版本記錄](https://www.hwinfo.com/version-history/) | 先做摘要，詳細資料按裝置展開；把來源、取樣時間及失效狀態放到數值旁。**授權要逐項查**：[官方授權](https://www.hwinfo.com/licenses/)區分個人／商業及功能；其[授權文件](https://www.hwinfo.com/files/license.pdf)限制第三方捆綁與規避限制，不應假設可以免費嵌入或繞過限制。 |
| **HWM — HWMonitor** | 集中顯示電壓、溫度、功率、風扇、負載和時脈等；依支援晶片取得資料。[CPUID 官方頁](https://www.cpuid.com/softwares/hwmonitor.html) | 同屏看不同類型的實測值，未知值明確標示；目前 MVP 只有部分項目。 |
| **CPUZ — CPU-Z** | CPU、快取、主機板和記憶體等分頁，SPD／XMP 資訊、核心／記憶體頻率；提供 CPU 基準與可保存的驗證資料。[CPUID 官方頁](https://www.cpuid.com/softwares/cpu-z.html)、[CPU-Z Validator](https://valid.cpuid.com/) | 靜態規格與即時數值分區；SPD 不應憑容量推測，也不能假設每台可讀。 |
| **GPUZ — GPU-Z** | 顯示顯卡與記憶體規格、提供 PCIe 通道負載測試及 BIOS 備份；可記錄感測值，部分支援的 GPU 有 PerfCap 原因欄位。[TechPowerUp 產品頁](https://www.techpowerup.com/gpuz/)、[下載頁與版本記錄](https://www.techpowerup.com/download/techpowerup-gpu-z/) | 區分實體 GPU、使用中的 API 後端和顯示輸出；測試名稱要講明目的。該頁亦列出商業套件再散布限制，不等同可捆綁。 |
| **AIDA — AIDA64 Extreme** | 系統資訊、感測器、自訂 SensorPanel、CPU／記憶體／儲存基準及穩定性測試。[官方產品頁](https://www.aida64.com/products/aida64-extreme) | 可配置卡片、同一報告結合配置與觀察；商業版本與授權不可假設。 |
| **LHM — LibreHardwareMonitor** | 覆蓋多類 CPU、GPU、主機板與儲存裝置的感測器；上游提供程式庫並指出部分感測器需管理員權限。[上游 README](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor#readme) | 可評估可選的 Windows 感測來源，但要隔離權限及故障；MPL-2.0 與第三方授權須審查，現在沒有整合。 |
| **OHM — Open Hardware Monitor** | 集中呈現溫度、風扇、電壓、負載及時脈等感測資訊。[上游介紹](https://openhardwaremonitor.org/) | 保留統一的感測器樹與機種相容性表；實際支援須逐機驗證。 |
| **CPUX — CPU-X** | 開源硬體資訊與監控，顯示 CPU、主機板等；支援的系統及架構以專案文件為準。[上游 README](https://github.com/TheTumultuousUnicornOfDarkness/CPU-X#readme) | 分離跨平台核心與平台提供者；它主要面向 Linux／FreeBSD，不能當作已完成 Windows 方案。上游 README 亦指出主要維護者已停止維護，依賴前需評估。 |

## 壓力、正確性與記憶體

| 代號與產品 | 已記錄的優點及來源 | 可借鑑／限制 |
| --- | --- | --- |
| **OCCT — OCCT** | 按核心選擇 CPU 測試、CPU＋RAM、模組化記憶體測試、GPU／VRAM、多測試並行與監控／休息階段。[功能頁](https://www.ocbase.com/personal-features)、[VRAM 說明](https://www.ocbase.com/support/stability-testing-configuration-vram) | 配方可選單項或組合，結果記錄實際測項；並行可能互相爭資源。VRAM 測試也有偵錯覆蓋限制，錯誤來源只能給出證據與可能性，不能保證定位。不同版本功能及授權不同。 |
| **P95 — Prime95** | 提供 CPU 壓力測試與資源限制設定。[GIMPS 官方下載頁](https://www.mersenne.org/download/) | 明確提供工作執行緒、時間和記憶體預算；失敗顯示工作負載及重現資訊，不把 CPU 高負載等同正確性通過。 |
| **YCR — y-cruncher** | 可擴展多執行緒計算、基準與壓力測試；官方介紹亦討論大型計算的錯誤偵測與檢查點。[官方頁](http://www.numberworld.org/y-cruncher/) | 長測試保存可解釋的階段／進度及驗證資訊；不要宣稱本產品已有相同演算法或吞吐能力。來源只提供 HTTP，勿從本文件自動下載程式。 |
| **FM — FurMark** | 集中做高負載 GPU 壓力與 OpenGL／Vulkan 圖形基準，提供命令列支援。[Geeks3D 官方頁](https://geeks3d.com/furmark/) | 將圖形負載、GPU 計算正確性、VRAM 驗證及分數分開；本 MVP 的小型 compute 檢查並非同等負載。 |
| **UNI — UNIGINE Superposition** | 圖形場景基準、循環穩定性測試，以及 GPU 溫度與時脈監看。[官方頁](https://benchmark.unigine.com/superposition) | 同步保存幀時間和可用溫度／時脈，呈現測試設定；不同版本／付費方案有差異。 |
| **M86 — MemTest86** | 獨立開機記憶體測試、不同測試與報告；支援條件下可回報 ECC。[官方介紹](https://www.memtest86.com/memtest86.html)、[ECC 說明](https://www.memtest86.com/ecc.htm)、[已知限制](https://www.memtest86.com/tech_blacklist-file.html) | 獨立開機與作業系統內測試應分清楚；開機環境亦有韌體與自身佔用記憶體限制，ECC 和各版本功能依平台而定。直接 HTTP 擷取官方站時遭 403；依可取得的搜尋索引摘要記錄，應在實作前再次核對原文。 |
| **M86P — Memtest86+** | 開源、開機測試、多種記憶體測試與可儲存報告；上游描述 SIMD／匯流排測試。[官方專案](https://www.memtest.org/) | 顯示測試覆蓋範圍、通過數及未覆蓋區域。開機映像和桌面產品需分開交付與驗證。 |
| **TM5 — TestMem5** | 這次未能從可確認的原作者官方來源核實功能或下載來源：疑似原站請求失敗；搜尋結果亦有獨立第三方及修改版。 | 列入待查，**不把任何設定檔／演算法聲稱為已驗證優點**，不提供不明鏡像下載；若日後取得可核實一手來源再補。 |

## 基準與真實工作負載

| 代號與產品 | 已記錄的優點及來源 | 可借鑑／限制 |
| --- | --- | --- |
| **CB — Cinebench** | 用渲染工作負載評估 CPU／GPU。[Maxon 官方頁](https://www.maxon.net/en/cinebench) | 「渲染任務時間」與穩定性錯誤檢查分開，必須記錄版本與負載。 |
| **GB — Geekbench** | 跨平台 CPU 單核／多核、GPU 計算與實際應用類型工作負載、結果比較。[官方頁](https://www.geekbench.com/) | 將易理解的單核／多核結果與具體負載放在一起；不同工作負載或版本不可直接比較。 |
| **3DM — 3DMark** | 不同圖形與系統基準、不同平台／情境的測試套件。[UL 官方頁](https://benchmarks.ul.com/3dmark) | 圖形基準先標註場景、API、解析度與設定；分數不能和其他產品直接互換。 |

## 儲存、效能診斷與風扇

| 代號與產品 | 已記錄的優點及來源 | 可借鑑／限制 |
| --- | --- | --- |
| **CDI — CrystalDiskInfo** | 呈現磁碟健康與 SMART 資訊、溫度、通電時間，以及可取得時的主機讀寫等資料，並提供狀態警示／事件。[基本資料](https://crystalmark.info/en/software/crystaldiskinfo/crystaldiskinfo-general-information/)、[狀態說明](https://crystalmark.info/en/software/crystaldiskinfo/crystaldiskinfo-health-status/)、[警報](https://crystalmark.info/en/software/crystaldiskinfo/crystaldiskinfo-resident/) | 展示原始值、廠商／協定及未知欄位；「健康」是來源的判讀，不能保證磁碟不會故障。 |
| **CDM — CrystalDiskMark** | 循序／隨機讀寫及混合工作負載；官方指出檔案大小、位置、資料與控制器會影響結果，寫入可消耗 SSD 壽命。[官方頁](https://crystalmark.info/en/software/crystaldiskmark/) | 用預設工作負載配合清楚標示檔案大小、隨機／循序、佇列深度、資料與寫入量；不要把單次結果當磁碟規格。 |
| **SMART — smartmontools** | `smartctl`／`smartd` 監測 ATA/SATA、SCSI/SAS、NVMe 的 SMART 資訊及裝置自我測試／日誌。[上游 README](https://github.com/smartmontools/smartmontools#readme)、[smartctl 手冊](https://github.com/smartmontools/smartmontools/blob/main/src/smartctl.8.in) | 協定與控制器可能阻擋欄位；讀取、查詢自我測試和改變裝置設定分開。若引用或分發工具須審查上游授權及版本。 |
| **DSPD — DiskSpd** | Windows 儲存負載產生器；可調整工作負載，亦有結構化設定／輸出。[Microsoft 專案](https://github.com/microsoft/diskspd)、[工作負載說明](https://learn.microsoft.com/en-us/azure/virtual-machines/disks-benchmarks) | 使用可重現的區塊大小、隨機性、佇列／工作者、預熱與測量時間；僅用本工具擁有的測試檔，不直接照搬高寫入量範例。 |
| **FIO — fio** | 利用工作檔定義、重現不同 I/O 工作負載。[上游說明](https://github.com/axboe/fio/blob/master/README.rst)、[完整文件](https://fio.readthedocs.io/en/latest/fio_doc.html) | 工作負載設定與結果一起保存；執行可能寫入目標，不能把外部設定檔當作無害。 |
| **HDS — HDDScan** | SMART 與多種磁碟讀取／表面檢查、測試圖。[官方頁](https://hddscan.com/) | 可以學習顯示測試範圍及錯誤位置；本產品不因此啟用 raw-device 寫入或宣稱無損。 |
| **LAT — LatencyMon** | 分析即時音訊相關的 DPC／ISR 執行時間、硬分頁錯誤，以及可能涉及的驅動／程序。[官方頁](https://www.resplendence.com/latencymon) | 把卡頓的系統事件證據和硬體溫度分開；量測能力、ETW 權限及時間關聯需另行實作驗證，不能自動斷言根因。 |
| **PE — Process Explorer** | 程序樹，以及所選程序使用的 DLL、handles 和搜尋。[Microsoft 官方頁](https://learn.microsoft.com/en-us/sysinternals/downloads/process-explorer) | 區分「哪個程序正在使用資源」和「硬體出錯」；只收集必要程序欄位並提供隱私控制。 |
| **PM — Windows Performance Monitor** | 可啟動效能監視、系統診斷報告與可靠性監視器。[Microsoft 文件](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/perfmon) | 提供深入診斷入口及可記錄的計數器，但不要把 OS 計數器視為硬體感測器。 |
| **WPR — Windows Performance Recorder** | 以 ETW 記錄系統／應用程式事件供分析，支援設定檔。[Microsoft 文件](https://learn.microsoft.com/en-us/windows-hardware/test/wpt/windows-performance-recorder) | 可評估按需啟動事件追蹤、保存範圍及清理；涉及權限、負擔、隱私和 ADK／部署條件。 |
| **PRES — PresentMon** | 利用 ETW 擷取與分析圖形應用的 CPU、GPU、顯示幀時間／延遲，可有即時 overlay。[上游 README](https://github.com/GameTechDev/PresentMon#readme) | 幀時間分佈與平均 FPS 分開。上游亦記錄 ETW 權限及非管理員程序資訊限制，必須顯示資料缺口。 |
| **MSI — MSI Afterburner** | 即時監控與遊戲畫面 OSD，另有風扇曲線／調校功能。[MSI 官方頁](https://www.msi.com/Landing/afterburner/graphics-cards) | 低干擾 overlay 有用；寫入風扇／時脈與唯讀監控是不同風險，不應預設修改。 |
| **FAN — Fan Control** | 首次設定引導、可保存設定檔、多溫度來源、風扇曲線及混合規則。[官方發行專案](https://github.com/Rem0o/FanControl.Releases#readme) | 多來源策略值得研究，但風扇控制需逐設備驗證、回復原狀及失效保護；上游指出程式本體為閉源，不能因 GitHub 頁存在就當作可直接整合的開源程式。 |

## 方法與未解問題

參考 [Craft Anything 的 study → build → check 流程](https://github.com/storytold/craft-anything#how-it-works)及 [REA 對證據和限制的描述](https://github.com/morluto/rea#readme)，本次人工執行「收集官方證據 → 比對目前 MVP → 明示落差和驗收」。兩個工具均**沒有執行、捆綁或被聲稱有分析過產品**。直接可取得的官方文件優先；官方頁的行銷文字已改寫為可核對的功能敘述，沒有獨立效能或覆蓋率測量。UEFI、ECC、SMART、GPU API、ETW 與風扇控制均需真實硬體、權限、驅動和授權逐項驗證；產品清單的空白及未能核實的 TM5 不代表功能不存在。
