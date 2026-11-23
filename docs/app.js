(() => {
  const REPO = "thodark/Boogie-AI-Assistant";

  const THEMES = [
    { id: "obsidian", short: "Obsidian", name: "Neon Obsidian", swatches: ["#946cff", "#26e0f5", "#40f2b8"] },
    { id: "aurora", short: "Aurora", name: "Aurora Prism", swatches: ["#ff4ecd", "#26e8ff", "#ffd646"] },
    { id: "sakura", short: "Sakura", name: "Cyber Sakura", swatches: ["#ff5cac", "#ce62ff", "#ffb0da"] },
    { id: "solar", short: "Solar", name: "Solar Flare", swatches: ["#ff722e", "#ffce48", "#ff3a5c"] },
    { id: "emerald", short: "Emerald", name: "Emerald Pulse", swatches: ["#20f098", "#18d6e8", "#a4ff48"] },
    { id: "arctic", short: "Arctic", name: "Arctic Frost", swatches: ["#42aeff", "#84f4ff", "#a4aeff"] },
    { id: "crimson", short: "Crimson", name: "Crimson Phantom", swatches: ["#ff385e", "#a43aff", "#ff8676"] },
    { id: "vapor", short: "Vapor", name: "Vaporwave Neon", swatches: ["#ff42ce", "#10eeff", "#ffe058"] },
  ];

  const I18N = {
    vi: {
      "nav.download": "Tải về",
      "nav.setup": "Cài đặt",
      "nav.commands": "Cách nói lệnh",
      "nav.performance": "Hiệu năng",
      "nav.changelog": "Changelog",
      "nav.contact": "Liên hệ",
      "nav.cta": "Tải ngay",
      "hero.badge": "Mới: 3 mức đồ họa, 8 chủ đề màu & Trí nhớ thông minh",
      "hero.title": 'Trợ lý AI giọng nói <span class="gradient-text">kiểu JARVIS</span> ngay trên Windows của bạn.',
      "hero.subtitle": "Tải về là chạy ngay — không cần cài đặt phức tạp, không cần biết code. Nói chuyện tự nhiên bằng tiếng Việt hoặc tiếng Anh với phản hồi ~0.58 giây, nhìn thấy màn hình, điều khiển 43 tác vụ Windows và nhớ những gì bạn kể.",
      "hero.downloadZip": "Tải cho Windows (ZIP — Khuyên dùng)",
      "hero.downloadExe": "Bản Portable 1 file (boogie.exe)",
      "hero.trust1": "Windows 10 / 11 (64-bit)",
      "hero.trust2": "Không cần Rust, Cargo hay Python",
      "hero.trust3": "Chạy Portable không cần cài",
      "hero.trust4": "Miễn phí & Mã nguồn mở (MIT)",
      "hero.tryThemes": "Thử đổi 8 bộ màu có sẵn trong Boogie:",
      "stats.startup": "Mở cửa sổ tức thì",
      "stats.ttfb": "Độ trễ phản hồi giọng nói",
      "stats.ram": "RAM thực tế siêu nhẹ",
      "stats.tools": "Công cụ điều khiển Windows",
      "download.eyebrow": "TẢI XUỐNG CHÍNH THỨC",
      "download.title": "Chọn bản phù hợp với bạn",
      "download.desc": "Tất cả bản phát hành đều được biên dịch tự động trực tiếp từ mã nguồn công khai trên GitHub Actions, không chứa phần mềm quảng cáo.",
      "download.recommended": "Khuyên dùng",
      "download.portable": "1 File duy nhất",
      "download.zipDesc": "Gói Portable đầy đủ gồm <code>boogie.exe</code>, mẫu cấu hình, hướng dẫn và giấy phép MIT. Chỉ cần giải nén vào một thư mục cố định (như <code>D:\\Boogie\\</code>) và mở chạy.",
      "download.exeDesc": "Bản chạy thẳng 1 tệp duy nhất, không cần giải nén. Tự động tạo thư mục <code>config/</code> bên cạnh file để lưu cấu hình, API key và bộ nhớ trên máy bạn.",
      "download.zipBtn": "Tải gói ZIP",
      "download.exeBtn": "Tải boogie.exe",
      "sha.title": "Kiểm tra mã toàn vẹn SHA-256",
      "sha.desc": "Đối chiếu mã SHA-256 sau khi tải để chắc chắn 100% file nguyên bản từ GitHub Releases và không bị chỉnh sửa.",
      "sha.fileCol": 'Tệp phát hành (<span id="sha-release-tag">latest</span>)',
      "common.copy": "Sao chép",
      "common.copied": "Đã sao chép!",
      "setup.eyebrow": "HƯỚNG DẪN LẦN ĐẦU",
      "setup.title": "Bắt đầu dùng chỉ trong 2 phút",
      "setup.desc": "Không cần gõ dòng lệnh. Làm đúng 4 bước bên dưới là bạn có thể trò chuyện với Boogie ngay.",
      "setup.s1Title": "Tải & giải nén vào thư mục cố định",
      "setup.s1Desc": "Tải <code>Boogie-Windows-x64.zip</code>, bấm chuột phải chọn <strong>Extract All</strong> vào một thư mục cố định (ví dụ <code>D:\\Boogie</code>) để Boogie lưu cấu hình và trí nhớ lâu dài.",
      "setup.s2Title": "Mở boogie.exe (Lưu ý Windows SmartScreen)",
      "setup.s2Desc": 'Click đúp <code>boogie.exe</code>. Nếu Windows hiện bảng xanh <em>"Windows protected your PC"</em> (vì ứng dụng mã nguồn mở miễn phí chưa mua chứng chỉ ký số EV), hãy bấm <strong>More info (Thông tin khác) → Run anyway (Vẫn chạy)</strong>.',
      "setup.s3Title": "Lấy Gemini API Key miễn phí",
      "setup.s3Desc": 'Mở <a href="https://aistudio.google.com/apikey" target="_blank" rel="noopener">Google AI Studio</a>, đăng nhập tài khoản Google → bấm <strong>Create API key</strong> → copy đoạn mã bắt đầu bằng <code>AIza...</code>. Key chỉ lưu ngay trên máy tính của bạn.',
      "setup.s4Title": "Dán Key, chọn đồ họa & bắt đầu nói!",
      "setup.s4Desc": "Trên màn hình <strong>BOOGIE SETUP</strong>, dán key vào ô <strong>Gemini API Key</strong>, chọn mức đồ họa phù hợp máy, chọn bộ màu yêu thích và giọng đọc → bấm <strong>Save &amp; Connect</strong> là xong!",
      "setup.gfxTitle": "Khuyến nghị chọn mức đồ họa Orb theo cấu hình máy",
      "setup.gfxSub": "Bạn có thể đổi trực tiếp lúc mở lần đầu hoặc bấm nút bánh răng ⚙ Settings bất cứ lúc nào để xem thử ngay (live preview).",
      "setup.tierLow": "Dành cho laptop văn phòng, máy yếu, Intel HD/UHD đời cũ. Dùng shader cầu 3D giải tích không vòng lặp — siêu mượt, ít nóng máy nhưng vẫn có hiệu ứng kính lỏng, gợn sóng theo giọng nói và vòng quỹ đạo 3D.",
      "setup.tierMed": "Cân bằng giữa đẹp và mượt cho laptop tầm trung, Intel Iris Xe hoặc AMD Radeon tích hợp. Giữ raymarching 3D thật với lõi plasma và bụi sao.",
      "setup.tierHigh": "Bản đẹp nhất cho PC / Laptop có card đồ họa rời (GTX/RTX/RX) hoặc iGPU mạnh. Raymarching 64 bước đầy đủ, ánh sáng studio 3 điểm và tán sắc cầu vồng.",
      "cmd.eyebrow": "HƯỚNG DẪN NÓI LỆNH",
      "cmd.title": "Nói gì để Boogie hiểu & làm chuẩn?",
      "cmd.desc": "Boogie hiểu tiếng Việt và tiếng Anh tự nhiên — không cần thuộc câu lệnh cứng nhắc. Dưới đây là những mẫu câu rõ ràng nhất cho từng nhóm tính năng (bấm vào mẫu câu để sao chép).",
      "cmd.searchPlaceholder": "Tìm nhanh tính năng (vd: màn hình, nhạc, hẹn giờ, nhớ...)",
      "tips.title": "Mẹo dùng Boogie thông minh & mượt nhất",
      "tips.t1Title": "Phòng ồn? Bật Push-to-Talk (Space)",
      "tips.t1Desc": "Mở ⚙ Settings → tích <em>Push-to-Talk (hold Space)</em>. Chỉ cần giữ phím <code>Space</code> khi nói, thả ra Boogie trả lời ngay — không lo tạp âm hay tiếng quạt làm nhiễu.",
      "tips.t2Title": "Ngắt lời tức thì (< 20 ms)",
      "tips.t2Desc": "Boogie đang nói dài quá? Cứ nói đè lên hoặc bấm nút vuông <strong>Stop</strong> trên thanh điều khiển bên dưới để dừng ngay lập tức.",
      "tips.t3Title": "Xem lại phụ đề & quản lý Trí nhớ",
      "tips.t3Desc": "Bấm thẳng vào dòng phụ đề bên dưới quả cầu để mở khung lịch sử hội thoại. Muốn xem/xóa thứ Boogie đang nhớ về mình, mở ⚙ <strong>Settings → Memory</strong>.",
      "tips.t4Title": "Nói chuyện tâm sự & đùa thoải mái",
      "tips.t4Desc": "Từ v0.2.0, Boogie đổi giọng điệu tự nhiên theo ngữ cảnh: làm việc gọn gàng, nói chuyện phiếm biết tung hứng dí dỏm, và lắng nghe ấm áp khi bạn tâm sự.",
      "sec.eyebrow": "AN TOÀN & QUYỀN RIÊNG TƯ",
      "sec.title": "Bạn luôn nắm quyền kiểm soát",
      "sec.c1Title": "Hỏi xác nhận trước thao tác nguy hiểm",
      "sec.c1Desc": "Xóa file, ghi file, chạy lệnh hệ thống thay đổi máy, tắt/khởi động lại PC, tắt ứng dụng hay gửi tin nhắn đều hiện bảng <strong>Approve / Deny</strong> trên màn hình — Boogie không bao giờ tự ý làm lén.",
      "sec.c2Title": "API Key & Ký ức lưu 100% trên máy bạn",
      "sec.c2Desc": "Không có máy chủ trung gian. API key (<code>api_keys.json</code>) và bộ nhớ (<code>memory.json</code>) nằm hoàn toàn trong thư mục máy tính của bạn và chỉ kết nối thẳng tới Google Gemini.",
      "sec.c3Title": "Chống Prompt Injection & Ghi nhật ký Audit",
      "sec.c3Desc": "Nội dung đọc từ web/PDF/clipboard được cách ly bằng thẻ <code>UNTRUSTED</code> để trang web lạ không thể ra lệnh điều khiển máy bạn. Mọi xác nhận nhạy cảm đều lưu trong <code>~/.boogie/audit.log</code>.",
      "sec.c4Title": "Bảo vệ Quota Gemini API thông minh",
      "sec.c4Desc": "Tóm tắt ký ức tự động chạy bằng <code>gemini-2.5-flash-lite</code> (tách quota với Live API), có thời gian nghỉ 10 phút, fallback cục bộ nếu hết quota và có thể bật/tắt tùy ý trong Settings.",
      "perf.eyebrow": "HIỆU NĂNG THỰC TẾ",
      "perf.title": "Vì sao viết hoàn toàn bằng Rust?",
      "perf.desc": "Không kẹp trình duyệt Chromium nặng nề như Electron, không cần cài môi trường Python hàng trăm MB — Boogie khởi động trong nháy mắt và ăn rất ít RAM.",
      "perf.chart1": "Tốc độ khởi động, RAM & Dung lượng",
      "perf.chart2": "Độ trễ phản hồi giọng nói trực tiếp",
      "cl.eyebrow": "LỊCH SỬ PHIÊN BẢN",
      "cl.title": "Changelog",
      "cl.latest": "Mới nhất",
      "faq.eyebrow": "HỖ TRỢ & LIÊN HỆ",
      "faq.title": "Câu hỏi thường gặp & Góp ý",
      "contact.bugTitle": "Báo lỗi & Đề xuất tính năng",
      "contact.bugDesc": "Gặp lỗi hoặc muốn thêm tính năng mới? Mở một GitHub Issue — tác giả sẽ xem và cập nhật trực tiếp.",
      "contact.emailTitle": "Liên hệ qua Email",
      "contact.emailDesc": "Gửi góp ý, báo lỗi (đính kèm <code>boogie.log</code>) hoặc trao đổi hợp tác trực tiếp với tác giả.",
      "contact.discordTitle": "Nhắn trực tiếp qua Discord",
      "contact.discordDesc": "Cần hỗ trợ nhanh cài đặt hoặc góp ý tính năng mới? Bấm vào thẻ này để sao chép username Discord.",
      "contact.authorTitle": "Tác giả — Suyz Nguyen",
      "contact.authorDesc": "Dự án mã nguồn mở phát triển bởi Suyz Nguyen. Xem mã nguồn Rust hoặc tặng 1 Star ủng hộ trên GitHub!",
      "contact.adviceTitle": "💡 Khuyến nghị khi gửi báo lỗi để được hỗ trợ nhanh nhất:",
      "contact.a1": "Cho biết phiên bản Windows (10 hay 11) và phiên bản Boogie bạn đang dùng (xem góc phải cửa sổ Settings).",
      "contact.a2": "Đính kèm file nhật ký <code>config/boogie.log</code> (file này chỉ ghi log lỗi kỹ thuật, <strong>không chứa</strong> Gemini API key của bạn).",
      "contact.a3": "Mô tả ngắn bạn vừa nói câu gì hoặc bấm vào đâu trước khi xảy ra lỗi.",
      "footer.credit": "Tạo bởi Suyz Nguyen · Giấy phép MIT",
      "footer.top": "Lên đầu trang ↑",
    },
    en: {
      "nav.download": "Download",
      "nav.setup": "Setup",
      "nav.commands": "Voice Guide",
      "nav.performance": "Benchmarks",
      "nav.changelog": "Changelog",
      "nav.contact": "Contact",
      "nav.cta": "Download",
      "hero.badge": "New: 3 Graphics Tiers, 8 Color Themes & Smart Memory",
      "hero.title": 'A <span class="gradient-text">JARVIS-style</span> real-time voice AI assistant for Windows.',
      "hero.subtitle": "Download and run instantly — no installer, no Rust or Python, no coding required. Talk naturally in English or Vietnamese with ~0.58s response speed, live screen vision, 43 Windows automation tools, and smart long-term memory.",
      "hero.downloadZip": "Download for Windows (ZIP — Recommended)",
      "hero.downloadExe": "Portable Single File (boogie.exe)",
      "hero.trust1": "Windows 10 / 11 (64-bit)",
      "hero.trust2": "No Rust, Cargo, or Python needed",
      "hero.trust3": "Portable — zero installation",
      "hero.trust4": "Free & Open-Source (MIT)",
      "hero.tryThemes": "Preview Boogie's 8 built-in color themes:",
      "stats.startup": "Window ready cold start",
      "stats.ttfb": "Avg voice response TTFB",
      "stats.ram": "Private RAM footprint",
      "stats.tools": "Native Windows tools",
      "download.eyebrow": "OFFICIAL DOWNLOADS",
      "download.title": "Choose your Windows package",
      "download.desc": "Every release binary is automatically built from public source code on GitHub Actions — clean, portable, and ad-free.",
      "download.recommended": "Recommended",
      "download.portable": "Single File",
      "download.zipDesc": "Complete portable bundle with <code>boogie.exe</code>, example config, README, and MIT License. Extract into any folder (e.g. <code>D:\\Boogie\\</code>) and double-click to run.",
      "download.exeDesc": "Standalone single executable with zero extraction needed. Automatically creates a local <code>config/</code> folder next to the file for your settings and memory.",
      "download.zipBtn": "Download ZIP Bundle",
      "download.exeBtn": "Download boogie.exe",
      "sha.title": "Verify Release Integrity (SHA-256)",
      "sha.desc": "Compare your downloaded file's SHA-256 hash to guarantee it came untouched from official GitHub Releases.",
      "sha.fileCol": 'Release file (<span id="sha-release-tag">latest</span>)',
      "common.copy": "Copy",
      "common.copied": "Copied!",
      "setup.eyebrow": "FIRST-TIME SETUP",
      "setup.title": "Up and talking in 2 minutes",
      "setup.desc": "No terminal commands required. Follow these 4 quick steps to launch Boogie.",
      "setup.s1Title": "Download & extract anywhere",
      "setup.s1Desc": "Download <code>Boogie-Windows-x64.zip</code>, right-click → <strong>Extract All</strong> into a permanent folder (like <code>D:\\Boogie</code>) so Boogie remembers your settings and memory across sessions.",
      "setup.s2Title": "Run boogie.exe (Windows SmartScreen)",
      "setup.s2Desc": 'Double-click <code>boogie.exe</code>. If Windows Defender SmartScreen shows <em>"Windows protected your PC"</em> (normal for unsigned open-source releases), click <strong>More info → Run anyway</strong>.',
      "setup.s3Title": "Get a free Gemini API Key",
      "setup.s3Desc": 'Open <a href="https://aistudio.google.com/apikey" target="_blank" rel="noopener">Google AI Studio</a>, sign in with Google → click <strong>Create API key</strong> → copy your <code>AIza...</code> key. It stays locally on your PC.',
      "setup.s4Title": "Paste key, pick theme & start talking!",
      "setup.s4Desc": "In <strong>BOOGIE SETUP</strong>, paste your <strong>Gemini API Key</strong>, choose a graphics tier for your PC, pick your favorite color theme & voice, and click <strong>Save &amp; Connect</strong>.",
      "setup.gfxTitle": "Recommended Graphics Quality by Hardware",
      "setup.gfxSub": "Choose on first launch or switch anytime via the top-right ⚙ Settings button with instant live preview.",
      "setup.tierLow": "Best for office laptops, older PCs & Intel HD/UHD integrated GPUs. Zero-loop analytical 3D glass shader — silky smooth and cool while keeping live ripples, glowing plasma core & 3D orbital ring.",
      "setup.tierMed": "Balanced visuals & speed for everyday laptops, Intel Iris Xe, or AMD integrated graphics. True 3D raymarching with swirling plasma and cosmic dust.",
      "setup.tierHigh": "Maximum visual fidelity for PCs & laptops with dedicated GPUs (GTX/RTX/RX) or strong iGPUs. Full 64-step 3D raymarching, 3-point studio speculars & chromatic dispersion.",
      "cmd.eyebrow": "VOICE COMMAND COOKBOOK",
      "cmd.title": "What to say so Boogie nails it",
      "cmd.desc": "Boogie understands natural Vietnamese and English — no rigid syntax required. Browse clear, proven phrases for each feature group (click any phrase to copy).",
      "cmd.searchPlaceholder": "Search features (e.g. screen, music, timer, memory...)",
      "tips.title": "Pro tips for the smoothest experience",
      "tips.t1Title": "Noisy room? Enable Push-to-Talk (Space)",
      "tips.t1Desc": "Open ⚙ Settings → check <em>Push-to-Talk (hold Space)</em>. Hold <code>Space</code> while speaking and release for instant responses with zero background-noise triggers.",
      "tips.t2Title": "Instant barge-in interrupt (< 20 ms)",
      "tips.t2Desc": "Want Boogie to stop mid-sentence? Just speak over it or hit the square <strong>Stop</strong> button on the floating bottom dock.",
      "tips.t3Title": "Transcript drawer & Memory Manager",
      "tips.t3Desc": "Click the live subtitles under the orb to slide open the conversation drawer. Open ⚙ <strong>Settings → Memory</strong> to inspect or delete stored facts & session summaries.",
      "tips.t4Title": "Banter & real conversations welcome",
      "tips.t4Desc": "As of v0.2.0, Boogie shifts tone naturally — crisp and fast on tasks, playful and witty in casual chats, and genuinely warm when you just want to talk.",
      "sec.eyebrow": "SECURITY & PRIVACY",
      "sec.title": "You always stay in control",
      "sec.c1Title": "On-screen approval for sensitive actions",
      "sec.c1Desc": "Deleting or writing files, mutating shell commands, restarting/shutting down your PC, killing apps, or sending messages always pop up an <strong>Approve / Deny</strong> card first.",
      "sec.c2Title": "API Key & Memory stay 100% local",
      "sec.c2Desc": "Zero third-party relay servers. Your API key (<code>api_keys.json</code>) and memory (<code>memory.json</code>) stay on your PC and connect straight to Google's Gemini endpoint.",
      "sec.c3Title": "Prompt Injection Defense & Audit Log",
      "sec.c3Desc": "External web, PDF, and clipboard content is isolated inside <code>UNTRUSTED</code> markers so malicious pages can't hijack desktop actions. Approved/denied actions are logged to <code>~/.boogie/audit.log</code>.",
      "sec.c4Title": "Quota-safe AI Memory Consolidation",
      "sec.c4Desc": "Background session summaries run on lightweight <code>gemini-2.5-flash-lite</code> (separate quota bucket from Live API) with a 10-minute cooldown, silent local fallback, and a Settings toggle.",
      "perf.eyebrow": "REAL BENCHMARKS",
      "perf.title": "Why pure Rust?",
      "perf.desc": "No bundled Chromium browser eating 800 MB RAM, no giant Python + Qt runtime folder — just one tiny native executable that launches in a blink.",
      "perf.chart1": "Cold Startup, RAM & Disk Footprint",
      "perf.chart2": "Real-Time Voice-to-Voice Latency",
      "cl.eyebrow": "RELEASE HISTORY",
      "cl.title": "Changelog",
      "cl.latest": "Latest",
      "faq.eyebrow": "HELP & CONTACT",
      "faq.title": "FAQ & Feedback",
      "contact.bugTitle": "Report a Bug or Request a Feature",
      "contact.bugDesc": "Found an issue or want a new desktop tool? Open a GitHub Issue and the author will review it directly.",
      "contact.emailTitle": "Contact via Email",
      "contact.emailDesc": "Send feedback, bug reports (with <code>boogie.log</code>), or collaboration inquiries directly to the creator.",
      "contact.discordTitle": "Chat on Discord",
      "contact.discordDesc": "Need quick setup help or want to suggest an idea? Click this card to copy the Discord username.",
      "contact.authorTitle": "Creator — Suyz Nguyen",
      "contact.authorDesc": "Open-source Rust project created by Suyz Nguyen. Audit the source code or leave a Star on GitHub!",
      "contact.adviceTitle": "💡 Tips when reporting an issue so we can fix it faster:",
      "contact.a1": "Include your Windows version (10 or 11) and Boogie version (shown in the top-right corner of Settings).",
      "contact.a2": "Attach <code>config/boogie.log</code> (contains only runtime diagnostics — <strong>never</strong> your Gemini API key).",
      "contact.a3": "Briefly describe what you said or clicked right before the issue happened.",
      "footer.credit": "Created by Suyz Nguyen · MIT License",
      "footer.top": "Back to top ↑",
    },
  };

  const COMMAND_CATEGORIES = [
    { id: "all", vi: "Tất cả", en: "All" },
    { id: "apps", vi: "App & Web", en: "Apps & Web" },
    { id: "pc", vi: "Điều khiển PC", en: "PC Control" },
    { id: "vision", vi: "Nhìn màn hình", en: "Screen Vision" },
    { id: "memory", vi: "Trí nhớ AI", en: "Smart Memory" },
    { id: "daily", vi: "Nhắn tin & Hẹn giờ", en: "Messages & Timers" },
    { id: "media", vi: "Nhạc & Steam", en: "Music & Steam" },
    { id: "files", vi: "File & Hệ thống", en: "Files & System" },
  ];

  const COMMAND_CARDS = [
    {
      cat: "apps",
      icon: "🚀",
      confirm: false,
      vi: {
        title: "Mở ứng dụng & trang web",
        desc: "Tự tìm & mở bất kỳ app Windows nào (Chrome, Brave, Discord, Zalo, VS Code, Spotify, Word, Steam...) hoặc mở thẳng trang web trên trình duyệt thật của bạn.",
        examples: [
          "Mở Discord với Spotify giúp mình",
          "Mở Chrome vào YouTube đi",
          "Mở trang github.com bằng Brave",
          "Chuyển sang cửa sổ VS Code giúp mình",
        ],
        tip: "Mẹo: Nói rõ tên trình duyệt nếu bạn muốn mở bằng trình duyệt riêng (Brave / Chrome / Edge / Firefox).",
      },
      en: {
        title: "Launch Apps & Websites",
        desc: "Finds, focuses, or opens any installed Windows app (Chrome, Brave, Discord, VS Code, Spotify, Steam...) or opens URLs directly in your real browser with your logins.",
        examples: [
          "Open Discord and Spotify",
          "Open Chrome and go to YouTube",
          "Open github.com in Brave",
          "Switch to Visual Studio Code",
        ],
        tip: "Tip: Mention a specific browser (Brave / Chrome / Edge / Firefox) whenever you don't want your default.",
      },
    },
    {
      cat: "pc",
      icon: "🎛️",
      confirm: true,
      vi: {
        title: "Điều khiển máy tính & Hoàn tác (Undo)",
        desc: "Tăng giảm âm lượng, độ sáng, Dark Mode, quản lý cửa sổ/tab, chụp ảnh màn hình, đổi hình nền, đọc/copy clipboard và hoàn tác lại thao tác vừa làm.",
        examples: [
          "Giảm độ sáng màn hình xuống một chút",
          "Chỉnh âm lượng lên 60 phần trăm",
          "Bật chế độ tối cho Windows đi",
          "Hoàn tác lại thao tác vừa rồi giúp mình",
        ],
        tip: "Các lệnh nhạy cảm (khóa máy, tắt màn hình, bật/tắt Wi-Fi, tắt hoặc khởi động lại PC) luôn hỏi xác nhận trên màn hình.",
      },
      en: {
        title: "PC Controls, Clipboard & Undo",
        desc: "Adjust volume, brightness, dark mode, window/tab shortcuts, screenshots, wallpaper, clipboard read/write, and undo reversible adjustments.",
        examples: [
          "Lower the screen brightness a bit",
          "Set volume to 60 percent",
          "Turn on Windows dark mode",
          "Undo what you just changed",
        ],
        tip: "High-impact actions (lock screen, sleep display, toggle Wi-Fi, restart, shutdown) always require on-screen approval.",
      },
    },
    {
      cat: "vision",
      icon: "👁️",
      confirm: false,
      vi: {
        title: "Nhìn màn hình (Real Screen Vision)",
        desc: "Boogie không tự quay lén màn hình liên tục — chỉ khi bạn nhờ nhìn, nó mới chụp 1 khung hình hiện tại để đọc lỗi, tóm tắt trang web hoặc hỗ trợ bạn.",
        examples: [
          "Nhìn màn hình xem lỗi đỏ trong code này là gì",
          "Xem giúp mình bảng tính đang mở trên màn hình",
          "Đọc và tóm tắt bài viết đang hiện trên màn hình đi",
          "Xem màn hình thứ hai đang mở gì giúp mình",
        ],
        tip: "Mẹo: Dùng từ \"nhìn màn hình\" hoặc \"xem màn hình\" để Boogie gọi công cụ vision ngay lập tức.",
      },
      en: {
        title: "Real-Time Screen Vision",
        desc: "Boogie never watches your screen silently — only when you ask, it captures a single frame to read errors, inspect apps, or summarize what's open.",
        examples: [
          "Look at my screen and explain this error",
          "Check the spreadsheet I currently have open",
          "Summarize the article on my screen right now",
          "Look at my second monitor and tell me what's there",
        ],
        tip: "Tip: Say \"look at my screen\" or \"check my screen\" so Boogie triggers screen capture immediately.",
      },
    },
    {
      cat: "memory",
      icon: "🧠",
      confirm: false,
      vi: {
        title: "Trí nhớ thông minh (Lưu, Nhớ lại & Quên)",
        desc: "Tự ghi nhớ sở thích, dự án, bạn bè và tóm tắt các cuộc trò chuyện cũ. Hỗ trợ tìm tiếng Việt không dấu, hỏi lại chuyện hôm trước hoặc bảo Boogie quên thông tin cũ.",
        examples: [
          "Nhớ giúp mình là thứ Sáu tuần này mình nộp đồ án",
          "Hôm qua mình với bạn đang bàn dở chuyện gì nhỉ?",
          "Mình từng bảo thích uống gì ấy nhỉ?",
          "Quên chuyện thích cà phê đen đi, giờ mình uống bạc xỉu",
        ],
        tip: "Bạn có thể xem toàn bộ ký ức và xóa thủ công bất cứ lúc nào trong ⚙ Settings → tab Memory.",
      },
      en: {
        title: "Smart Memory (Save, Recall & Forget)",
        desc: "Remembers preferences, projects, people, and past session summaries with accent-tolerant search. Ask about previous talks or tell Boogie to forget outdated info.",
        examples: [
          "Remember that my thesis deadline is this Friday",
          "What were we talking about yesterday?",
          "Do you remember what coffee I like?",
          "Forget that I like black coffee — I prefer lattes now",
        ],
        tip: "Inspect or delete individual memories and past sessions anytime inside ⚙ Settings → Memory tab.",
      },
    },
    {
      cat: "daily",
      icon: "⏰",
      confirm: true,
      vi: {
        title: "Nhắn tin, Nhắc lịch & Hẹn giờ đếm ngược",
        desc: "Đặt nhắc nhở hệ thống Windows (vẫn báo kể cả khi đã tắt Boogie), đếm ngược thời gian, hỏi giờ và gửi tin nhắn qua WhatsApp, Telegram, Discord, Messenger.",
        examples: [
          "Đếm ngược 15 phút nữa nhắc mình tắt bếp",
          "Đặt lịch nhắc lúc 7 giờ sáng mai đi học",
          "Xem danh sách lịch nhắc đang có đi",
          "Nhắn Telegram cho Minh là tối nay 8 giờ họp nhé",
        ],
        tip: "Gửi tin nhắn luôn hiện hộp thoại xác nhận người nhận + nội dung trước khi bấm gửi.",
      },
      en: {
        title: "Messages, OS Reminders & Timers",
        desc: "Set Windows Task Scheduler reminders (fires even when Boogie is closed), countdown timers, check date/time, and send messages via WhatsApp, Telegram, Discord, or Messenger.",
        examples: [
          "Set a 15-minute timer to check the oven",
          "Remind me tomorrow at 7 AM to head to class",
          "List all my active reminders",
          "Send a Telegram message to Minh saying meeting at 8 PM",
        ],
        tip: "Sending a message always pops up an on-screen confirmation showing recipient and text first.",
      },
    },
    {
      cat: "apps",
      icon: "🌐",
      confirm: false,
      vi: {
        title: "Tìm kiếm Web, Thời tiết, Tin tức & Vé máy bay",
        desc: "Tra cứu nhanh trên DuckDuckGo, xem thời tiết từng thành phố, đọc tin nóng, theo dõi chủ đề tin tức định kỳ ngầm và tra cứu chuyến bay trên Google Flights.",
        examples: [
          "Thời tiết Hà Nội hôm nay thế nào?",
          "Tìm tin tức mới nhất về AI tuần này",
          "Tìm vé máy bay từ Sài Gòn đi Đà Nẵng ngày mai",
          "Theo dõi giúp mình tin tức mới về Rust mỗi tiếng",
        ],
        tip: "Mẹo: Nói rõ tên thành phố hoặc ngày đi (hôm nay / ngày mai / ngày cụ thể) để ra kết quả chuẩn ngay.",
      },
      en: {
        title: "Web Search, Weather, News & Flights",
        desc: "Fast DuckDuckGo web lookup, live city weather, latest headlines, background topic news monitoring, and Google Flights search.",
        examples: [
          "What's the weather in Hanoi today?",
          "Get the latest news headlines about AI",
          "Find flights from Saigon to Da Nang tomorrow",
          "Monitor news updates about Rust for me",
        ],
        tip: "Tip: Mention the exact city or date (today / tomorrow / YYYY-MM-DD) for instant first-try results.",
      },
    },
    {
      cat: "media",
      icon: "🎵",
      confirm: false,
      vi: {
        title: "Phát nhạc YouTube, Video & Quản lý game Steam",
        desc: "Tìm và mở thẳng bài hát/video trên YouTube (tự bỏ qua Shorts), dừng/phát nhạc hệ thống, và kiểm tra/cài đặt/cập nhật game trong thư viện Steam.",
        examples: [
          "Mở nhạc Lofi chill trên YouTube giúp mình",
          "Tạm dừng nhạc lại đi",
          "Xem danh sách game đã cài trên Steam",
          "Cập nhật game Counter-Strike trên Steam giúp mình",
        ],
        tip: "Mẹo: Với yêu cầu liên quan tới game Steam, cứ nhắc chữ \"Steam\" để Boogie dùng công cụ Steam chuyên dụng.",
      },
      en: {
        title: "YouTube Music, Media & Steam Games",
        desc: "Search and play songs/videos on YouTube (automatically skips Shorts), control system playback, and list, install, or update games in Steam.",
        examples: [
          "Play some chill lofi music on YouTube",
          "Pause the music",
          "List my installed Steam games",
          "Update Counter-Strike on Steam",
        ],
        tip: "Tip: Mention \"Steam\" for game library actions so Boogie routes straight to the native Steam tool.",
      },
    },
    {
      cat: "files",
      icon: "📁",
      confirm: true,
      vi: {
        title: "Đọc PDF / Excel, Quản lý File, QR & Tắt Boogie",
        desc: "Tìm file trên máy, đọc nội dung file chữ / PDF / Excel, tạo mã QR, xem CPU/RAM/ổ cứng, đóng app đang treo và tắt gọn chính Boogie khi dùng xong.",
        examples: [
          "Kiểm tra CPU, RAM và dung lượng ổ cứng giúp mình",
          "Đọc file PDF báo cáo ở Desktop rồi tóm tắt lại",
          "Tạo mã QR cho đường link https://github.com/thodark",
          "Tắt Boogie đi nhé (chỉ đóng app, không tắt máy)",
        ],
        tip: "Ghi/xóa/di chuyển file, kill tiến trình hoặc lệnh shell thay đổi hệ thống luôn cần bạn bấm Approve.",
      },
      en: {
        title: "Files, PDF/Excel, QR Codes & System Status",
        desc: "Search files, read text/PDF/Excel documents, generate QR codes, check CPU/RAM/disk usage, close frozen apps, and gracefully shut down Boogie.",
        examples: [
          "Check my CPU, RAM, and disk space",
          "Read the PDF report on my Desktop and summarize it",
          "Generate a QR code for https://github.com/thodark",
          "Shut down Boogie (closes the app only, not your PC)",
        ],
        tip: "Writing, deleting, or moving files, killing processes, and mutating shell commands always ask for confirmation.",
      },
    },
  ];

  const CHANGELOG = {
    v020: {
      vi: [
        "<strong>3 mức đồ họa Orb (Low / Medium / High):</strong> Chọn ngay từ màn hình Setup hoặc trong Settings với live preview, FPS thích ứng (30 / 45 / 60 FPS) và MSAA thích ứng (0x / 2x / 4x). Bản <code>Low (Lite)</code> nhẹ hơn ~15× cho máy yếu nhưng vẫn giữ orb kính 3D phát sáng và vòng quỹ đạo.",
        "<strong>8 bộ màu giao diện có sẵn:</strong> Neon Obsidian, Aurora Prism (cầu vồng động), Cyber Sakura, Solar Flare, Emerald Pulse, Arctic Frost, Crimson Phantom và Vaporwave Neon — đổi mượt đồng bộ cả 3D Orb lẫn toàn bộ UI.",
        "<strong>Hệ thống Trí nhớ Thông minh (Smart Memory):</strong> Tìm kiếm tiếng Việt có/không dấu xuyên cả fact lẫn lịch sử hội thoại cũ, thêm tool <code>forget_memory</code>, ghi file atomic chống hỏng dữ liệu, lưu an toàn khi bấm nút X, tóm tắt phiên ngầm bằng <code>gemini-2.5-flash-lite</code> có bảo vệ quota, và tab <strong>Memory</strong> trong Settings để xem/xóa 1 click.",
        "<strong>Tính cách tự nhiên, ấm áp &amp; biết đùa hơn:</strong> Giữ sự gọn gàng sắc sảo kiểu JARVIS khi làm việc, nhưng tung hứng vui vẻ khi tán gẫu và ấm áp chân thành khi bạn tâm sự.",
      ],
      en: [
        "<strong>3 Orb Graphics Quality Tiers (Low / Medium / High):</strong> Selectable in Setup and Settings with live preview, adaptive FPS (30 / 45 / 60 FPS), and adaptive MSAA (0x / 2x / 4x). <code>Low (Lite)</code> cuts GPU load ~15× for weak/integrated GPUs while keeping glowing 3D liquid glass visuals and the orbital ring.",
        "<strong>8 Live-Preview Color Themes:</strong> Neon Obsidian, Aurora Prism (animated rainbow), Cyber Sakura, Solar Flare, Emerald Pulse, Arctic Frost, Crimson Phantom, and Vaporwave Neon — smoothly morphing both the 3D orb and UI accents.",
        "<strong>Smart Long-Term Memory &amp; Memory Manager Tab:</strong> Accent-tolerant multi-word search across both saved facts and episodic history, new <code>forget_memory</code> tool, atomic writes, guaranteed save on window close, quota-safe background AI consolidation (<code>gemini-2.5-flash-lite</code>, 10m cooldown), and an in-app <strong>Memory</strong> tab in Settings.",
        "<strong>Warmer, Wittier Personality:</strong> Keeps Boogie's sharp JARVIS-style competence on commands while opening up playful humor during casual chats and genuine warmth during personal talks.",
      ],
    },
    v010: {
      vi: [
        "<strong>Phát hành bản Windows x64 đầu tiên:</strong> Đóng gói tự động <code>Boogie-Windows-x64.zip</code>, <code>boogie.exe</code> portable và <code>SHA256SUMS.txt</code> qua GitHub Actions.",
        "<strong>Lõi Rust thuần siêu nhanh:</strong> Mở cửa sổ ~137 ms, độ trễ phản hồi giọng nói ~580 ms, RAM thực tế ~33 MB cùng giao diện 3D Liquid Glass GLSL 60 FPS.",
        "<strong>Setup lần đầu &amp; Settings ngay trong app:</strong> Không cần sửa file JSON thủ công; hỗ trợ đổi nóng API key, giọng đọc, mic/loa, Push-to-Talk (<code>Space</code>) và ghi log <code>boogie.log</code>.",
        "<strong>42 công cụ Windows &amp; Lớp bảo mật:</strong> Mở app native, điều khiển trình duyệt, nhìn màn hình, hộp thoại xác nhận thao tác nguy hiểm và chống prompt injection.",
      ],
      en: [
        "<strong>First Prebuilt Windows x64 Release:</strong> Automated GitHub Actions build shipping <code>Boogie-Windows-x64.zip</code>, standalone <code>boogie.exe</code>, and <code>SHA256SUMS.txt</code>.",
        "<strong>Pure Rust Voice Runtime &amp; 3D Orb:</strong> ~137 ms cold window ready, ~580 ms avg audio TTFB, ~33 MB private RAM, and a 60 FPS raymarched GLSL Liquid Glass orb.",
        "<strong>Built-in Setup &amp; Live Settings Modal:</strong> Zero manual JSON editing — hot-reload API key, voice, mic/speaker devices, Push-to-Talk (<code>Space</code>), and persistent <code>boogie.log</code> diagnostics.",
        "<strong>42 Native Windows Tools &amp; Security Gates:</strong> Native app launcher, browser control, real screen vision, on-screen confirmation dialogs, and prompt-injection defense.",
      ],
    },
  };

  const FAQS = {
    vi: [
      {
        q: "Tại sao Windows Defender SmartScreen hiện cảnh báo xanh khi mở boogie.exe?",
        a: "Vì Boogie là phần mềm mã nguồn mở miễn phí, chưa mua chứng chỉ ký số thương mại EV (tốn hàng trăm USD/năm), nên Windows mặc định cảnh báo với mọi file .exe mới tải từ Internet. Bạn có thể tự kiểm tra mã SHA-256 ở mục Tải về hoặc xem toàn bộ mã nguồn công khai trên GitHub, rồi bấm <strong>More info → Run anyway</strong>.",
      },
      {
        q: "Máy laptop văn phòng không có card rời dùng có bị giật lag không?",
        a: "Từ bản <strong>v0.2.0</strong>, ngay màn hình Setup đầu tiên (hoặc trong ⚙ Settings) bạn chỉ cần chọn mức đồ họa <strong>Low (Lite)</strong>. Chế độ này loại bỏ hoàn toàn vòng lặp raymarching nặng, chạy ở 30 FPS cực mát máy mà quả cầu 3D vẫn sáng đẹp mượt mà.",
      },
      {
        q: "Gemini API key miễn phí có dùng để nói chuyện lâu thoải mái được không?",
        a: "Có! Google AI Studio cung cấp hạn mức miễn phí cho Gemini API. Boogie cũng tối ưu rất kỹ để tiết kiệm quota: giới hạn dung lượng khối nhớ trong prompt, tự xoay vòng phiên gọn gàng, và tính năng tóm tắt ký ức ngầm chạy trên model siêu nhẹ <code>gemini-2.5-flash-lite</code> (tách quota với Live API, tối đa 1 lần / 10 phút) — nếu chạm hạn mức sẽ tự fallback về lưu cục bộ mà không làm gián đoạn cuộc trò chuyện.",
      },
      {
        q: "Boogie không nghe thấy giọng mình hoặc tự phản hồi khi có tiếng ồn?",
        a: "Hãy bấm nút bánh răng ⚙ <strong>Settings</strong>, chọn đúng thiết bị <strong>Mic</strong> và <strong>Speaker</strong> đang dùng. Nếu bạn đang ở phòng ồn hoặc dùng loa ngoài âm lượng lớn, hãy bật <strong>Push-to-Talk (hold Space)</strong> để chỉ thu âm khi bạn nhấn giữ phím Space.",
      },
      {
        q: "Dữ liệu hội thoại, API key và bộ nhớ của mình được lưu ở đâu?",
        a: "Tất cả đều nằm cục bộ trong thư mục <code>config/</code> nằm ngay cạnh file <code>boogie.exe</code> (gồm <code>api_keys.json</code>, <code>memory.json</code>, <code>long_term.json</code> và <code>boogie.log</code>). Nếu muốn chuyển sang máy khác, chỉ cần copy thư mục Boogie; nếu muốn xóa sạch trí nhớ, mở ⚙ Settings → tab <strong>Memory</strong> → <strong>Clear All Memory</strong>.",
      },
    ],
    en: [
      {
        q: "Why does Windows Defender SmartScreen warn me when opening boogie.exe?",
        a: "Because Boogie is a free open-source project without a paid commercial EV code-signing certificate, Windows warns on newly downloaded executables by default. Verify the SHA-256 hash in the Download section (or audit the open source on GitHub), then click <strong>More info → Run anyway</strong>.",
      },
      {
        q: "Will Boogie run smoothly on a regular laptop without a dedicated GPU?",
        a: "Yes! Starting in <strong>v0.2.0</strong>, pick <strong>Low (Lite)</strong> right in the first-run Setup screen (or anytime in ⚙ Settings). It switches to a zero-loop analytical 3D glass shader at 30 FPS — ~15× lighter on integrated GPUs while still looking sleek and audio-reactive.",
      },
      {
        q: "Can I chat comfortably for long sessions on a single Gemini API key?",
        a: "Yes. Boogie uses budgeted memory prompts so token usage stays compact, and its optional background AI memory summarizer uses lightweight <code>gemini-2.5-flash-lite</code> (separate quota bucket from Gemini Live) with a strict 10-minute cooldown and silent local fallback if rate-limited — plus you can toggle it off anytime in Settings.",
      },
      {
        q: "Boogie isn't hearing my mic or gets triggered by background noise?",
        a: "Open ⚙ <strong>Settings</strong> and explicitly select your active <strong>Mic</strong> and <strong>Speaker</strong> devices. In noisy rooms or with loud speakers, enable <strong>Push-to-Talk (hold Space)</strong> so Boogie listens only while you hold Space.",
      },
      {
        q: "Where are my API key, settings, and memories stored?",
        a: "Everything is stored locally inside the <code>config/</code> folder next to <code>boogie.exe</code> (<code>api_keys.json</code>, <code>memory.json</code>, <code>long_term.json</code>, and <code>boogie.log</code>). You can inspect or wipe stored memories anytime via ⚙ Settings → <strong>Memory</strong> tab.",
      },
    ],
  };

  let currentLang = localStorage.getItem("boogie_lang") || (navigator.language && navigator.language.toLowerCase().startsWith("vi") ? "vi" : "en");
  let currentTheme = localStorage.getItem("boogie_theme") || "obsidian";
  let currentCat = "all";
  let searchQuery = "";
  let toastTimer = null;

  function t(key) {
    return (I18N[currentLang] && I18N[currentLang][key]) || I18N.vi[key] || key;
  }

  function showToast(msg) {
    const el = document.getElementById("toast");
    if (!el) return;
    el.textContent = msg;
    el.classList.add("show");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove("show"), 2000);
  }

  async function copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(t("common.copied"));
    } catch {
      const ta = document.createElement("textarea");
      ta.value = text;
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      document.body.removeChild(ta);
      showToast(t("common.copied"));
    }
  }

  function applyTheme(themeId) {
    const found = THEMES.find((x) => x.id === themeId) || THEMES[0];
    currentTheme = found.id;
    document.documentElement.setAttribute("data-theme", found.id);
    localStorage.setItem("boogie_theme", found.id);
    const nameEl = document.getElementById("active-theme-name");
    if (nameEl) nameEl.textContent = found.name;
    document.querySelectorAll(".theme-chip").forEach((btn) => {
      btn.classList.toggle("active", btn.dataset.themeId === found.id);
    });
  }

  function renderThemePills() {
    const wrap = document.getElementById("theme-pills");
    if (!wrap) return;
    wrap.innerHTML = THEMES.map(
      (th) => `
      <button type="button" class="theme-chip ${th.id === currentTheme ? "active" : ""}" data-theme-id="${th.id}" title="${th.name}">
        <span class="swatches">
          ${th.swatches.map((c) => `<span class="swatch-dot" style="background:${c}"></span>`).join("")}
        </span>
        <span>${th.short}</span>
      </button>`
    ).join("");
    wrap.querySelectorAll(".theme-chip").forEach((btn) => {
      btn.addEventListener("click", () => applyTheme(btn.dataset.themeId));
    });
  }

  function renderCommandTabs() {
    const wrap = document.getElementById("cmd-tabs");
    if (!wrap) return;
    wrap.innerHTML = COMMAND_CATEGORIES.map(
      (c) =>
        `<button type="button" class="cmd-tab ${c.id === currentCat ? "active" : ""}" data-cat="${c.id}" role="tab" aria-selected="${c.id === currentCat}">${c[currentLang]}</button>`
    ).join("");
    wrap.querySelectorAll(".cmd-tab").forEach((btn) => {
      btn.addEventListener("click", () => {
        currentCat = btn.dataset.cat;
        renderCommandTabs();
        renderCommandCards();
      });
    });
  }

  function renderCommandCards() {
    const grid = document.getElementById("cmd-grid");
    if (!grid) return;
    const q = searchQuery.trim().toLowerCase();
    const confirmLabel = currentLang === "vi" ? "Có xác nhận" : "Confirm gate";
    const copyLabel = currentLang === "vi" ? "Copy" : "Copy";

    const filtered = COMMAND_CARDS.filter((card) => {
      if (currentCat !== "all" && card.cat !== currentCat) return false;
      if (!q) return true;
      const d = card[currentLang];
      const hay = `${d.title} ${d.desc} ${d.examples.join(" ")} ${d.tip}`.toLowerCase();
      return hay.includes(q);
    });

    if (filtered.length === 0) {
      grid.innerHTML = `<div class="glass-card" style="grid-column:1/-1;text-align:center;color:var(--text-soft)">${
        currentLang === "vi" ? "Không tìm thấy tính năng phù hợp — thử từ khóa khác nhé." : "No matching commands found — try another keyword."
      }</div>`;
      return;
    }

    grid.innerHTML = filtered
      .map((card) => {
        const d = card[currentLang];
        return `
        <article class="glass-card cmd-card">
          <div class="cmd-card-head">
            <div class="cmd-card-title">
              <span class="cmd-icon">${card.icon}</span>
              <h3>${d.title}</h3>
            </div>
            ${card.confirm ? `<span class="confirm-pill">${confirmLabel}</span>` : ""}
          </div>
          <p class="cmd-card-desc">${d.desc}</p>
          <div class="cmd-examples">
            ${d.examples
              .map(
                (ex) => `
              <button type="button" class="cmd-quote" data-quote="${ex.replace(/"/g, "&quot;")}">
                <span>“${ex}”</span>
                <span class="quote-copy">${copyLabel}</span>
              </button>`
              )
              .join("")}
          </div>
          <p class="cmd-tip">${d.tip}</p>
        </article>`;
      })
      .join("");

    grid.querySelectorAll(".cmd-quote").forEach((btn) => {
      btn.addEventListener("click", () => copyText(btn.dataset.quote));
    });
  }

  function renderChangelog() {
    const v020 = document.getElementById("cl-v020");
    const v010 = document.getElementById("cl-v010");
    if (v020) v020.innerHTML = CHANGELOG.v020[currentLang].map((li) => `<li>${li}</li>`).join("");
    if (v010) v010.innerHTML = CHANGELOG.v010[currentLang].map((li) => `<li>${li}</li>`).join("");
  }

  function renderFaqs() {
    const list = document.getElementById("faq-list");
    if (!list) return;
    list.innerHTML = FAQS[currentLang]
      .map(
        (item, idx) => `
      <div class="faq-item ${idx === 0 ? "open" : ""}">
        <button type="button" class="faq-q" aria-expanded="${idx === 0}">
          <span>${item.q}</span>
          <span class="faq-plus">+</span>
        </button>
        <div class="faq-a">${item.a}</div>
      </div>`
      )
      .join("");
    list.querySelectorAll(".faq-q").forEach((btn) => {
      btn.addEventListener("click", () => {
        const parent = btn.closest(".faq-item");
        const isOpen = parent.classList.toggle("open");
        btn.setAttribute("aria-expanded", String(isOpen));
      });
    });
  }

  function applyLanguage(lang) {
    currentLang = lang === "en" ? "en" : "vi";
    document.documentElement.lang = currentLang;
    document.title =
      currentLang === "en"
        ? "Boogie — Native Rust Voice AI Assistant for Windows"
        : "Boogie — Trợ Lý AI Giọng Nói Cho Windows | Native Rust Voice AI";
    localStorage.setItem("boogie_lang", currentLang);

    document.querySelectorAll(".lang-btn").forEach((b) => {
      b.classList.toggle("active", b.dataset.lang === currentLang);
    });

    document.querySelectorAll("[data-i18n]").forEach((el) => {
      const key = el.getAttribute("data-i18n");
      if (I18N[currentLang][key]) el.innerHTML = I18N[currentLang][key];
    });

    document.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
      const key = el.getAttribute("data-i18n-placeholder");
      if (I18N[currentLang][key]) el.setAttribute("placeholder", I18N[currentLang][key]);
    });

    renderCommandTabs();
    renderCommandCards();
    renderChangelog();
    renderFaqs();
    syncReleaseTagText();
  }

  let latestTagText = "v0.2.0";
  function syncReleaseTagText() {
    const shaTag = document.getElementById("sha-release-tag");
    if (shaTag) shaTag.textContent = latestTagText;
  }

  function formatMB(bytes) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  async function hydrateLatestRelease() {
    try {
      const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
        headers: { Accept: "application/vnd.github+json" },
      });
      if (!res.ok) throw new Error("release fetch failed");
      const rel = await res.json();
      const tag = rel.tag_name || "v0.2.0";
      latestTagText = tag;

      ["nav-version", "hero-release-tag", "card-zip-version", "card-exe-version", "sha-release-tag"].forEach((id) => {
        const el = document.getElementById(id);
        if (el) el.textContent = tag;
      });

      const assets = Array.isArray(rel.assets) ? rel.assets : [];
      const zip = assets.find((a) => a.name === "Boogie-Windows-x64.zip");
      const exe = assets.find((a) => a.name === "boogie.exe");
      const sha = assets.find((a) => a.name === "SHA256SUMS.txt");

      if (zip) {
        const sz = formatMB(zip.size);
        document.getElementById("hero-zip-size").textContent = sz;
        document.getElementById("card-zip-size").textContent = sz;
        document.getElementById("hero-zip-btn").href = zip.browser_download_url;
        document.getElementById("card-zip-link").href = zip.browser_download_url;
        if (zip.digest && zip.digest.startsWith("sha256:")) {
          document.getElementById("sha-zip-hash").textContent = zip.digest.slice(7);
        }
      }
      if (exe) {
        const sz = formatMB(exe.size);
        document.getElementById("hero-exe-size").textContent = sz;
        document.getElementById("card-exe-size").textContent = sz;
        document.getElementById("hero-exe-btn").href = exe.browser_download_url;
        document.getElementById("card-exe-link").href = exe.browser_download_url;
        if (exe.digest && exe.digest.startsWith("sha256:")) {
          document.getElementById("sha-exe-hash").textContent = exe.digest.slice(7);
        }
      }
      if (sha) {
        document.getElementById("sha-file-link").href = sha.browser_download_url;
      }
    } catch {
      // Fallback gracefully if GitHub API rate-limits anonymous client IP
      const zipEl = document.getElementById("sha-zip-hash");
      const exeEl = document.getElementById("sha-exe-hash");
      if (zipEl && zipEl.textContent.includes("...")) {
        zipEl.textContent = "See SHA256SUMS.txt on GitHub Releases ↗";
      }
      if (exeEl && exeEl.textContent.includes("...")) {
        exeEl.textContent = "See SHA256SUMS.txt on GitHub Releases ↗";
      }
    }
  }

  function init() {
    renderThemePills();
    applyTheme(currentTheme);
    applyLanguage(currentLang);
    hydrateLatestRelease();

    document.querySelectorAll(".lang-btn").forEach((btn) => {
      btn.addEventListener("click", () => applyLanguage(btn.dataset.lang));
    });

    const searchInput = document.getElementById("cmd-search");
    if (searchInput) {
      searchInput.addEventListener("input", (e) => {
        searchQuery = e.target.value;
        renderCommandCards();
      });
    }

    document.querySelectorAll(".copy-btn").forEach((btn) => {
      btn.addEventListener("click", () => {
        const target = document.getElementById(btn.dataset.copyTarget);
        if (target) copyText(target.textContent.trim());
      });
    });

    document.querySelectorAll("[data-copy-value]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const val = btn.getAttribute("data-copy-value");
        if (val) copyText(val);
      });
    });

    const toggle = document.getElementById("menu-toggle");
    const navLinks = document.getElementById("nav-links");
    if (toggle && navLinks) {
      toggle.addEventListener("click", () => {
        const open = navLinks.classList.toggle("open");
        toggle.setAttribute("aria-expanded", String(open));
      });
      navLinks.querySelectorAll("a").forEach((a) => {
        a.addEventListener("click", () => navLinks.classList.remove("open"));
      });
    }
  }

  document.addEventListener("DOMContentLoaded", init);
})();
