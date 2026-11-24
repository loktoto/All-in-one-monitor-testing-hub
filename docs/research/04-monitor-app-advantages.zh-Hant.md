# 監控軟件優點彙整：逐項證據、可借鑑設計與本機對照

整理日期：2026-11-17。這份文件只做一件事：把清單上每個監控軟件的**優點**逐項記錄下來，
寫清楚「佢好在邊」、「我哋可以偷乜」、「代價／限制係乜」，最後對照本專案現時真正做得到的部分。

本節原先是公開資料研究；後續已用兩個工具分析公開開源材料，詳見[工具研究記錄](../reference-study.md#tool-assisted-study-2026-11-17)，並沒有逆向任何商業二進位檔：

- [Craft Anything](https://github.com/storytold/craft-anything) 的研究紀律：先 *study*（只靠公開
  文件與可觀察行為收集證據）→ *build*（獨立實作，不搬對方程式碼）→ *check*（逐項比對並列出
  未符合的地方）；每個結論都要有證據來源、覆蓋範圍與限制，缺失就寫「unknown」，不靠推測補洞。
- [REA](https://github.com/morluto/rea) 的證據紀律：保留資料來源、涵蓋範圍和限制；摘要可再展開詳情。此處按同一方式將官方記錄、本機實測與未知範圍分開。

因此下表的「優點」全部是**官方產品頁／官方文件／上游 repo 可查到的能力**，不是我們的實機量測；
「可借鑑」是設計判斷，不代表原產品有這樣做，也不代表本專案已實作（現況一律以
[目前能力清單](../capabilities.md) 與程式碼為準）。商業產品未下載、未安裝、未執行。

## 摘要（粵語）

呢批軟件其實分三種「勁」：**HWiNFO／HWMonitor 勁在感測器廣度同 min/max**、**Afterburner＋RTSS
同 PresentMon 勁在遊戲內即時 overlay 同幀時間證據**、**CPU-Z／GPU-Z／CrystalDiskInfo 勁在單一
硬件的靜態身份同健康細節**；NZXT CAM 勁在「唔使你部機係佢品牌都用得」同迷你 overlay；
工作管理員／PowerToys 勁在「零安裝、即刻睇到、仲可以釘住喺工具列」。

我哋部 Hardware Hub 已經偷到嘅係：SMC 直讀 → 16 個溫度感測器、每個都有 current/min/max 同
reset（HWMonitor 那招）、Details 規格表（CPU-Z／GPU-Z／CrystalDiskInfo 那招）、LIVE/STALE
新鮮度標籤（HWiNFO 的失效語意）。**仲差**：電壓／瓦數（HWiNFO 最強那塊）、真正的 S.M.A.R.T.
即時原始屬性（可匯入報告）、遊戲內 OSD，以及 GPU／磁碟逐程序歸因。CPU／記憶體程序排行和桌面浮動窗已加入。

## 一、綜合全方位硬體監控

| 產品 | 官方記錄的監控項目 | 優點（好在邊） | 可借鑑的設計 | 限制／代價 |
| --- | --- | --- | --- | --- |
| **HWiNFO** · [官方介紹](https://www.hwinfo.com/about/) · [版本記錄](https://www.hwinfo.com/version-history/) | CPU、GPU、RAM、主機板、硬碟溫度、時脈、電壓、功耗、風扇轉速等幾乎所有感測器 | 感測器覆蓋業界最廣；摘要（Summary）先給一眼重點，再逐裝置展開；每個感測器有目前／最低／最高／平均與重設；支援記錄、報告、警報，以及共享記憶體讓其他程式（含 RTSS）讀取 | 「先摘要、後細節」的資訊層級；把**來源 key、取樣時間、失效狀態**貼在數值旁邊；min/max 明確標成「觀察期極值」 | 感測器多到需要分組與搜尋，否則難讀；部分讀值要驅動或管理員權限；授權分個人／商業，不可假設可免費捆綁 |
| **HWMonitor** · [CPUID 官方頁](https://www.cpuid.com/softwares/hwmonitor.html) | CPU（溫度／時脈／佔用率）、GPU、RAM、硬碟 S.M.A.R.T. 狀態與讀寫溫度 | 由 CPUID 開發，用**樹狀階層**列當前值／最小值／最大值，一屏睇完、零學習成本；未知項目直接留空而不是填 0 | 樹狀＋三欄（Value/Min/Max）就是最省認知成本的表達；把「硬體 → 子系統 → 感測器」的層級寫進資料模型，而不是用扁平 list 硬砌 | 沒有歷史圖表與記錄深度；S.M.A.R.T. 只給狀態字，不是原始屬性表 |
| **NZXT CAM** · [官方頁](https://nzxt.com/pages/cam) · [版本記錄](https://nzxt.com/pages/cam-release-notes) | CPU、GPU、RAM、網路流量、硬碟使用率與溫度 | 介面美感與資訊密度平衡好；**任何 Windows PC 都免費、免帳號**（不是只服務自家硬件）；Mini mode 把精簡 overlay 釘在桌面；CPU 溫度可選「核心平均」或「Package」，風扇曲線可選溫度來源 | 「可選感測來源」是誠實設計：同一個「CPU 溫度」其實有多個合理定義，讓使用者選並記住選擇；overlay 走「迷你、可釘住」而不是全屏 | 深度監控偏浅（無電壓／功耗細節）；風扇、RGB、LCD 控制要自家硬件；純 Windows |
| **LibreHardwareMonitor** · [上游 README](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor#readme) | CPU 負載、GPU 溫度、記憶體使用率、硬碟資訊與網路活動（跨 CPU／GPU／主機板／儲存） | 開源、可作為**程式庫**嵌入；不需常駐廠商服務即可取基礎感測值；社群持續補新晶片 | 以 provider 介面隔離「感測器從哪來」，一個 provider 失效不應拖死整個 UI（我哋 `sensors.rs` 已有 fallback 到 `sysinfo` 的骨架） | 部分感測器需管理員權限；MPL-2.0 與第三方 notices 要先審；晶片支援逐機而異 |
| **PC Hardware Monitor** | 清單只給了通用名稱 | **未能核實**：同名工具有多個，沒有可確認的一手官方來源，所以不列任何「已驗證優點」 | —（不把來源不明的功能寫進能力目錄） | 待有可核實的一手來源再補；在此之前不應拿它做設計依據 |

## 二、遊戲內即時表現與效能疊加（OSD）

| 產品 | 官方記錄的監控項目 | 優點（好在邊） | 可借鑑的設計 | 限制／代價 |
| --- | --- | --- | --- | --- |
| **MSI Afterburner + RTSS** · [官方頁](https://www.msi.com/Landing/afterburner/graphics-cards) · [RTSS](https://www.guru3d.com/download/rtss-rivatuner-statistics-server-download) | GPU 負載、VRAM 佔用、CPU 核心佔用率、RAM 使用量、FPS、風扇轉速與溫度 | PC 玩家事實標準：**可自選要顯示哪幾個感測器**、位置、字型、顏色；RTSS 提供幀時間圖與幀率限制；監控歷史可記錄成檔案；風扇曲線與超頻在同一個介面 | overlay 的價值在「**使用者自己揀顯示乜**」，不是固定面板；把 FPS 與 **frametime** 一起給（平均 FPS 會騙人）；記錄要能匯出成時間序列 | OSD 需要 hook 遊戲，可能與 anti-cheat 衝突；Windows only；設定項極多，預設體驗對新手不友善 |
| **Intel PresentMon** · [官方頁](https://game.intel.com/us/intel-presentmon) · [開源 repo](https://github.com/GameTechDev/PresentMon) | CPU／GPU 延遲、FPS、效能瓶頸分析 | 開源、可整合；**GPU Busy** 指標把 CPU 與 GPU 的平衡講清楚，比單看佔用率更接近「邊個係瓶頸」；overlay 支援即時折線圖與 histogram、percentile 與 rolling average；支援 DX9–12／OpenGL／Vulkan，多廠硬件 | 報告要同時給 **平均、percentile（1% low）與分佈**，不要只給一個數；overlay 用 preset（例如「Game Experience」）降低設定門檻；擷取結果存成可再分析的檔（CSV） | 只覆蓋圖形／遊戲路徑，不是整機監控；部分指標依驅動與 API 支援；overlay 與擷取是兩個用途，要分清 |

## 三、單項硬體深度檢視

| 產品 | 官方記錄的監控項目 | 優點（好在邊） | 可借鑑的設計 | 限制／代價 |
| --- | --- | --- | --- | --- |
| **CPU-Z** · [官方頁](https://www.cpuid.com/softwares/cpu-z.html) · [Validator](https://valid.cpuid.com/) | CPU 型號、核心時脈、快取、主機板型號、RAM 規格與記憶體時序（SPD） | 分頁式規格表資訊密度極高但可預期（CPU／Mainboard／Memory／SPD／Graphics／Bench）；**靜態規格與即時時脈分開**；Validator 讓結果可外部比對 | 規格表用固定分頁而不是一張長表；SPD 逐條 module 列（型號、part number、時序），讀不到就寫 unavailable | 不做監控歷史與警報；部分欄位依晶片組／權限而缺失；Validator 是自願上傳，涉及機器識別資料 |
| **GPU-Z** · [TechPowerUp 產品頁](https://www.techpowerup.com/gpuz/) | 顯示卡型號、GPU 核心頻率、VRAM 容量與類型、溫度、驅動版本 | 一屏講清「這張卡係邊張、用邊個 driver、跑緊乜 API」；可記錄感測器；有 PCIe 通道負載測試與 BIOS 備份；支援的卡會顯示 **PerfCap reason**（為何被限速） | 「為何不是全速」比「而家幾多度」更有診斷價值：把 throttle／限速原因當成第一級欄位；BIOS 備份這類破壞性操作要獨立、明確 | 只覆蓋顯示卡；部分欄位需廠商 API；商業再散布受限 |
| **CrystalDiskInfo** · [官方頁](https://crystalmark.info/en/software/crystaldiskinfo/) | HDD／SSD／NVMe 健康度、溫度、讀寫總量與 S.M.A.R.T. 數值 | 把 S.M.A.R.T. 由「一堆 hex」翻譯成**人看得明的屬性表**（ID、目前值、最差值、臨界值、原始值、狀態色）；健康狀態一眼判斷；溫度與總寫入量長期可見；可設溫度警報 | 原始值與判讀**同時**顯示，並標明判讀規則來源；健康狀態要有第三種（Caution/Unknown），不要只得「好／壞」 | 不同廠商 raw value 定義不一致，不能機械化解讀；NVMe 與 USB bridge 支援視情況；常駐讀取裝置需權限 |
| **Windows 工作管理員** · [Microsoft 支援](https://support.microsoft.com/windows) | CPU、RAM、磁碟、網路與 GPU 即時使用率及佔用程式排行 | **零安裝、零權限、永遠在**；有逐程序歸因（邊個 process 食緊 CPU／GPU／磁碟），這是所有監控工具最常被追問的一格；GPU 頁更細分 3D／Copy／Video 引擎 | 「總量 + 誰在用」要一起給；把程序排行當成診斷入口，而不是另一個 tab | 取樣粗、無歷史、無感測器（無溫度／電壓）；細節欄位依 Windows 版本 |
| **Microsoft PowerToys** · [Command Palette 文件](https://learn.microsoft.com/en-us/windows/powertoys/command-palette/overview) | CPU、RAM、GPU、網路（含網速）、磁碟、電池 | 開源；Command Palette 可即時查系統效能，並可把**個別指標釘在 Dock** 成為常駐細條；不用開一個大視窗 | 常駐顯示應該「一条一條指標自己釘」，而不是強迫整套 dashboard；提供「快速查一下」與「長期釘住」兩種使用模式 | 不是完整監控工具（無感測器深度、無歷史、無 S.M.A.R.T.）；釘在頂部的做法屬於 Command Palette Dock，不是獨立模組 |

## 四、對照本專案：已經偷到的、未偷的

| 來源優點 | 本專案現況 | 依據 |
| --- | --- | --- |
| HWMonitor 式樹狀 + current/min/max + reset | **已有**：AppleSMC 直讀，每個溫度列都有 Current / Min / Max，可 Reset min/max；min/max 明確標示為本次啟動觀察值 | `src/sensors.rs`、`src/main.rs`、[capabilities](../capabilities.md) |
| CPU-Z／GPU-Z／CrystalDiskInfo 式規格表 | **部分**：Details 頁有 CPU 快取／核心、記憶體 module、GPU driver 與 vendor/device ID、每磁碟區 S.M.A.R.T. 狀態、NVMe 型號／韌體／link／TRIM；**未有原始 S.M.A.R.T. 屬性表與 SPD 時序** | `src/specs.rs`、[capabilities](../capabilities.md) |
| HWiNFO 式失效語意（STALE／unknown） | **部分**：LIVE／STALE／WAITING 標籤與 `—`／Not available；**未有逐感測器時間戳與完整新鮮度模型** | `src/main.rs` |
| NZXT CAM 式「可選感測來源」 | **未有**：SMC key 直接顯示，未有「CPU 溫度用 package 定核心平均」的選擇 | — |
| Afterburner／PresentMon 式 OSD 與幀時間 | **部分**：有 CPU／RAM 桌面浮動窗及離線 PresentMon CSV 分析；沒有遊戲內 OSD 或即時 FPS 擷取 | — |
| 工作管理員式逐程序歸因 | **部分**：可搜尋與排序 CPU／記憶體使用量；GPU 與磁碟程序歸因未有 | `src/telemetry.rs`、`src/main.rs` |
| PowerToys 式可釘住單一指標 | **部分**：有 CPU／RAM 桌面浮動窗，未有單一指標自訂 | — |

## 五、如果要排先後：最值得偷的五樣

1. **S.M.A.R.T. 原始屬性表**（CrystalDiskInfo）——可匯入本機 smartctl JSON；自動、即時、已驗證裝置對應仍未有，並保留「unknown 就是 unknown」。
2. **電壓／功耗區塊**（HWiNFO）——SMC 有 key 但語意逐機型不同，做法應是 opt-in、逐 key 標註、明示 model-dependent，而不是塞進主表。
3. **每感測器新鮮度**（HWiNFO）——把 timestamp 與 provider key 放進 hover，STALE 由「整個 app 級」降到「每個值級」。
4. **frametime／percentile 報告格式**（PresentMon）——即使未有 overlay，報告也應給 1% low 與分佈，而不是單一平均。
5. **可釘住的迷你顯示**（CAM Mini mode／PowerToys Dock）——成本最低、對日常使用最有感的一項。

## 六、來源與可查證性

- 產品能力：HWiNFO、CPUID（HWMonitor／CPU-Z）、NZXT（CAM 頁與版本記錄）、TechPowerUp（GPU-Z）、CrystalMark（CrystalDiskInfo）、MSI／Guru3D（Afterburner、RTSS）、Intel（PresentMon 頁與 [GameTechDev/PresentMon](https://github.com/GameTechDev/PresentMon)）、Microsoft Learn（PowerToys Command Palette）、LibreHardwareMonitor 上游 README。
- 只核對了**頁面所述的產品能力**，沒有檢查每個版本差異，也沒有在 Windows 實機重現任何一項；本機是 macOS（見 [HANDOFF](../../HANDOFF.md) §3）。
- 連結可達性（2026-11-17，只檢查 HTTP 回應）：NZXT、CrystalDiskInfo、Intel PresentMon 返回 200；MSI 與 Guru3D 對自動請求返回 403（站點攔截機械存取），其在瀏覽器開啟的內容未在此重新核實。
- 「PC Hardware Monitor」與清單內的「Windows 內建工具」屬通用描述：前者無可核實一手來源，故不列優點；後者以工作管理員與 PowerToys 兩份官方文件作代表。
- 我哋自己的數字（例如 16 個溫度感測器）來自本機實測，與上表「官方聲稱」分開記錄，不混為一談。
