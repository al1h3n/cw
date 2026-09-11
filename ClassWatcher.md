Coding:
Skills to use: /caveman, /ponytail, /rust-best-practices, /anti-ui-slop, /best-practices, /tauri, /i-have-adhd

# Theory.
Skills to use: /marketing-mindset, /ponytail, /grill-me

### Abstract.
I'm going to develop a project that should have client and server (main host) app.
You need to develop a clear plan with small steps inlcuding testing for bugs to launch the application.
For now, call it "Co-watcher". Do not create any title for it yet, cause it's not developed yet and I'll probably have to make a research.
Assess the project as a future billionaire startup that will have a huge developers team and company. So do not obscure code.

### What is it about?
Screen watcher, open-source "AnyDesk".
For example, you're a teacher. There are cabinets of students in your cabinet and you don't want them to play.
A basic (free-tier) functionality is going to contain:
- Screen share (you will be able to pin other monitors with client-side program installed). That said, you can watch monitor of other student, including virtual desktops.
- Audio share (switch to enable/disable to avoid high network usage).
- Ability to use your mouse and keyboard to use other's PC. E.g. close a game. When pressing Windows, it should also receive that and not be intersected with host OS. Exit by something like Win+Esc or so.
- Ability to lock screen like in Family Link and other parental controls apps.
- Ability to immediately shut down all computers, or power off individual ones. E.g. an exam is happening and they'll be powered off.
- Make a wallpaper constant, so no-one would be able to change it.
- Save all files made by users temporary that will be deleted with one button. Data should be observable by host.
- Add other computers via ID or LAN (latter one still connects via ID cause network might suddenly change, so if you edit anything on student's PC it's going to apply when it will be connected to Wi-Fi and changes will be fetched).
- Enable/disable programs. E.g. disable all games (on free-tier there's a list of basic games and you can edit them manually).
- Edit lock screen when student first opens PC. Administrator (teacher), can edit background of it and shortcuts assigned for each PC. There should be terminal available in bottom-right corner with power controls. Terminal accepts custom commands (not like actual cmd) such as administrator request unlock (e.g. `unlock` makes you enter one-time code or constant code provided in next sections).
- When host connects, use black background to avoid additional resource usage to share pixels. Use smart, effective and contemporary algorithms to share video.
- Create screen recordings of all PCs or individual ones, with ability to set it up with specific time and date.
- Make all/individual screens show screen share (full or piece of it, or specific app screen share) of host. At that time, no keyboard or mouse shortcuts should work.
- Send custom audio/video files or recordings in real time with notification that you got something in real-time, like exam listening that can't be repeated.
- Update center, being able to freely update application from main deploy branch.

### Paid functions with subscription.
Although free-tier version is already good, paid will intersect AI usage.
You will be able to:
- Use AI to create brief abstact of lessons (for students or host if he wants them to be able to do it).
- Add own API key (like Claude, ChatGPT or Google AI Studio, DeepSeek etc) or use model in paid subscription (btw it's going to be random free model in HuggingFace).
- Make AI watch what's happening on screen (and you have to somehow optimize it to reduce resource usage and API token cost), and make it do actions based on context. E.g. host set up to close gaming/entertainment apps/browser tabs when AI sees it on screen for all students. Basically, custom AI tools.
- And evetything else with AI stuff.
- Adding more than 10 devices requires paid subscription as well. For schools and universities, custom plan will be applied, with 40 devices per cabinet for a required amount of money. For businesses - $2 per month for one device. And there can be thousands of them. You can suggest own price based on current economy state, even though they will be altered depending on the country.

### Programming language.
Rust. It should be used for all backend if that's possible.
If any dashboard will be used in site, or lock screen with shortcuts like in PC clubs, you can use Tauri (that also uses Rust) for web pages.
Other languages might be used, even though Rust is in preference.

### Platforms.
I would like to develop it for Windows first. But I know that it should be available on macOS and Linux as well.
The thing is, screen share, audio and other functionality might differ depends on OS.
Windows might use own SDK and API. Linux uses pipewire and pulseaudio etc.
You'll have to resolve all that.
About compatibility with older versions: if modern approaches are available, they're in precedence. But if you can add both (e.g. the one available for Windows 11 24H2+ for screen share and Windows 7 and older) do it please. More compatibility - more devices. Do not forget about macOS and Linux either! There can be BSDs, Arch Linux, Fedora, nixOS and even more!

### Privacy.
I know that viewing data is weird. But teachers have to somehow resolve gaming issue in classes.
You need to also create a secret code for administrator to login and temporary disable any watchers if errors will occur. (E.g. `GreatAdm1nistratorsAccessForOneMoreTime101`)

### Concerns.
I'm not sure what should happen if no internet is available. Seems like all host changes won't be received until internet is on? You should decide how to let it happen.

### Performance.
I guess all that stuff will make PC not so optimized. So, try to optimize it as much as possible. That's the reason why I gave you Rust as main language - it's modern and almost C++ level.

### How updates will work.
Once we develop a stable version, we're going to set up a git repo with automatic CI-CD agents that test code for errors or bugs (last one can be used via AI-based CI-CD testers).
After that, prod branch updates and update is available for others.
Something like `Hydra` in `nixpkgs` repo (I mean tool like that) will automatically build binaries to be added in Release section. Everything is automatic.
I'm not sure about versioning. Is it going to be X.Y.Z with semantic versioning or YYYY.MM.DD(-patch) format?
It's preferable to have multiple mirrors, such as GitHub, GitLab and Codeberg.

### Task for you.
Based on this information, set up a clear plan. I want this app to be optimized, be good and advanced. I also want to monetize it. A tutorial when app is started should be available, with ability to skip it.
Maybe there will be one binary, and if you open it you can select, whether the PC is client or server with ID to be later added.
You shouldn't start the project yet until you generate the plan. Create AGENTS.md in `"C:\Users\al1h3n\Downloads\ClassWatcher"` and use it as main path for all your code etc for main information to avoid forgetting instuctions after you made them. After evetything is ready, we will be able to start. Background information is given, so it's your start.