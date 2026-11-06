# hyprland-rust yol haritası: Hyprland'in tamamını Rust'a geçirmek

Hedef: Hyprland'in yerini birebir alabilen bir compositor. Aynı
`hyprland.conf`, aynı dispatcher'lar, `hyprctl` ve event socket uyumu.
waybar, hyprlock, hypridle ve xdg-desktop-portal-hyprland gibi araçlar
değişiklik yapılmadan çalışmalı.

Ölçek: Hyprland on binlerce satır C++ ve yıllardır geliştiriliyor. Bu plan
uzun soluklu. Aşamalar, her birinin sonunda kullanılabilir bir şey çıkacak
şekilde sıralandı.

## Temel kurallar

- **Hedef sürüm:** Hyprland'in tek bir etiketli sürümü seçilir ve wiki'si
  "spec" kabul edilir. Config sözdizimi sürümler arasında değişiyor
  (örneğin `windowrule` / `windowrulev2`). Bir sürüm bitince bir sonrakine
  geçilir.
- **Uyumluluk tablosu:** `docs/PARITY.md` dosyasında hedef sürümün her config
  seçeneği, dispatcher'ı, hyprctl komutu, IPC olayı ve Wayland protokolü
  satır satır durur. Durumlar: yok / kısmi / tam / test edildi. İlerleme
  buradan ölçülür.
- **Her özellik ayrı dal ve PR olur.** PR, CI yeşil olunca birleştirilir.
  Saf mantık (config, layout, IPC biçimi) Smithay'den bağımsız tutulur ve
  unit test'le korunur.
- **Port değil, yeniden yazım:** Hyprland'in kodu davranışı anlamak için
  okunur (BSD-3). Birebir çevrilmez. Altyapı Smithay.

## Aşama 0: Altyapı (diğer her şeyin önkoşulu)

- TTY backend'ini gerçek donanımda çalıştırmak, çıkan hataları düzeltmek.
- Boşta sürekli yeniden çizimi kaldırmak: kenarlık ve imleç elemanlarına
  kalıcı `Id` vermek, damage'ı doğru hesaplamak.
- Cargo workspace'e geçmek: `hypr-config` (hyprlang), `hypr-layout`,
  `hypr-ipc` (mesaj tipleri, JSON), `hyprctl` (ayrı binary), `compositor`.
- **Headless test backend'i:** gerçek ekran ve Xvfb olmadan çalışan bir
  backend. Surfaceless EGL veya pixman renderer üzerine kurulur. Test,
  compositor'ı açar, Wayland test istemcileri bağlar, pencere
  geometrisini ve ekran görüntüsünü kontrol eder. CI'daki entegrasyon
  testleri bunun üzerinde koşar.
- `docs/PARITY.md` tablosunu hedef sürümün wiki'sinden çıkarmak.

Bitti sayılır: TTY'de pencere açılıyor, CI'da headless entegrasyon testi
var, uyumluluk tablosu yazılmış.

## Aşama 1: Tek monitörde günlük kullanılabilir

- Çoklu monitör: CRTC başına bir `DrmOutput`, udev hotplug, `monitor=`
  kuralları (çözünürlük, yenileme hızı, konum, ölçek, transform, disable).
  Her monitörün kendi aktif workspace'i.
- Input ayarları: `input { kb_layout = tr, kb_variant, kb_options,
  repeat_rate/delay, sensitivity, accel_profile, natural_scroll,
  follow_mouse }`, `touchpad { }`, cihaza özel `device { }` blokları.
- İmleç: xcursor teması, istemcinin kendi imleç yüzeyi,
  cursor-shape protokolü, donanım imleç düzlemi.
- Temel protokoller: xdg-decoration (sunucu taraflı), layer-shell (waybar,
  rofi, mako), primary selection, data-control (clipboard yöneticileri),
  viewporter, fractional-scale, presentation-time, linux-dmabuf,
  xdg-activation, idle-inhibit, ext-idle-notify (hypridle),
  ext-session-lock (hyprlock), wlr-screencopy (grim, slurp).
- XWayland: Smithay'in `xwayland` özelliği ve X11 pencere yöneticisi.
  `xwayland { force_zero_scaling }` seçeneği.

Bitti sayılır: waybar, rofi, mako, hyprlock, hypridle, grim/slurp, Firefox
ve bir XWayland uygulaması (örneğin Steam) çalışıyor; bir gün boyunca günlük
kullanım mümkün.

## Aşama 2: IPC ve `hyprctl`

- Socket yolu: `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/` altında
  `.socket.sock` (istek) ve `.socket2.sock` (olay).
  `HYPRLAND_INSTANCE_SIGNATURE` ortam değişkeni ayarlanır.
- İstekler: `dispatch`, `keyword`, `reload`, `monitors`, `workspaces`,
  `activeworkspace`, `clients`, `activewindow`, `layers`, `devices`, `binds`,
  `getoption`, `cursorpos`, `version`, `kill`, `notify`, `switchxkblayout`,
  `seterror`, `configerrors`, `--batch` ve `-j` (JSON). JSON alan adları
  Hyprland'inkilerle birebir aynı olmalı.
- Olaylar: `workspace(v2)`, `focusedmon`, `activewindow(v2)`, `openwindow`,
  `closewindow`, `movewindow`, `createworkspace`, `destroyworkspace`,
  `fullscreen`, `monitoradded/removed`, `openlayer/closelayer`, `submap`,
  `changefloatingmode`, `urgent`, `windowtitle`, `configreloaded` vb.
- `hyprctl` Rust'ta ayrı bir binary olarak yazılır, çıktı biçimi aynı olur.

Bitti sayılır: waybar'ın `hyprland/workspaces` ve `hyprland/window`
modülleri çalışıyor; Hyprland için yazılmış hyprctl betikleri değişmeden
çalışıyor. JSON şemaları Hyprland'in çıktısına karşı test ediliyor.

## Aşama 3: Pencere yönetimi uyumu

- Bütün dispatcher'lar (yaklaşık 70 tane): `movewindow`, `swapwindow`,
  `resizeactive`, `moveactive`, `centerwindow`, `pin`, `focusmonitor`,
  `movecurrentworkspacetomonitor`, `swapactiveworkspaces`, `focusurgentorlast`,
  `focuscurrentorlast`, `pass`, `sendshortcut`, `global`, `setprop`,
  `fullscreenstate`, `dpms`, `alterzorder`, `tagwindow` vb.
- Workspace seçicileri: `e+1`, `m+1`, `r-1`, `name:`, `previous`, `empty`,
  `special:`. İsimli, kalıcı ve özel (scratchpad) workspace'ler,
  `workspace =` kuralları.
- Bind bayrakları: `bindl`, `bindr`, `binde`, `bindm` (fareyle taşıma ve
  boyutlandırma), `bindn`, `bindt`, `bindi`, `bindo`, `bindd`. Submap'ler.
- Kurallar: hedef sürümün sözdizimiyle `windowrule`, `layerrule`. Eşleşme
  ölçütleri (class, title, xwayland, floating, workspace, tag...) ve
  etkileri.
- Layout'lar: dwindle'ın bütün seçenekleri, master layout, gruplar (sekmeli
  pencereler ve groupbar).
- Davranışlar: pencere yutma (swallowing), `focus_on_activate`, urgent,
  aktif pencereyi üste getirme, odak geçmişi.

Bitti sayılır: Hyprland wiki'sindeki bütün dispatcher'lar ve bind türleri
uyumluluk tablosunda "test edildi" durumunda.

## Aşama 4: Config uyumu

- hyprlang'in tamamı: değişkenler, iç içe kategoriler, `source` glob'ları,
  hedef sürümün desteklediği koşullu ifadeler ve özel sözdizimi.
- Dosya değişince otomatik yeniden yükleme, `hyprctl reload`, çalışırken
  `hyprctl keyword` ile ayar değiştirme.
- Config hatalarını ekranın üstünde bir şerit olarak göstermek (Hyprland
  gibi).
- `env`, `exec`, `exec-once`, `exec-shutdown`, `gestures`, `misc`,
  `ecosystem`, `debug` kategorileri.
- `is_known_unimplemented` listesinin sıfırlanması: tanınan ama yok sayılan
  hiçbir seçenek kalmamalı.

Bitti sayılır: Hyprland'in varsayılan config'i ve topluluktan seçilmiş
birkaç büyük config hatasız yükleniyor. Parser fuzz testinden geçiyor.

## Aşama 5: Görsel efektler

Bütün efektler özel GLES shader'ları ve render elemanları olarak yazılır.

- Yuvarlak köşeler (`rounding`, `rounding_power`) ve açılı gradyan
  kenarlıklar.
- Gölgeler, `dim_inactive`, aktif ve pasif opaklık.
- Blur: dual kawase. Pass sayısı, size, noise, contrast, brightness,
  vibrancy, xray, popup ve layer blur. Performans için blur önbelleği
  şart.
- Animasyonlar: `bezier =` eğrileri, animasyon ağacı (`windows`,
  `windowsIn/Out/Move`, `layers`, `fade`, `border`, `borderangle`,
  `workspaces`, `specialWorkspace`). Stiller: `popin`, `slide`,
  `slidevert`, `slidefade`, `gnomed`.
- `screen_shader`.

Bitti sayılır: aynı config ile Hyprland'le yan yana alınan ekran
görüntüleri görsel olarak eşleşiyor. Headless backend'de referans görüntü
testleri var.

## Aşama 6: İleri seviye çıktı ve render

- Damage tracking'in her yerde doğru olması, doğrudan scanout.
- Kesirli ölçek, transform, ekran yansıtma (mirror).
- VRR, tearing-control, DPMS, gamma ve CTM kontrolü.
- Çoklu GPU (anvil'deki `GpuManager` / `MultiRenderer`), explicit sync
  (drm-syncobj).
- 10-bit renk, HDR ve color-management protokolü.

Bitti sayılır: yüksek yenileme hızlı ve çoklu GPU'lu (örneğin hibrit
laptop) makinelerde kare kaybı yok. Hyprland'e karşı karşılaştırmalı
performans ölçümleri yapılmış.

## Aşama 7: Protokol tamamlığı ve ekosistem

- Ekran paylaşımı: xdg-desktop-portal-hyprland için
  hyprland-toplevel-export ve ext-image-copy-capture; hyprland-global-shortcuts.
- Hyprland'e özel diğer protokoller: hyprland-focus-grab, hyprland-ctm-control,
  hyprland-surface.
- Giriş: text-input v3 ve input-method v2 (fcitx5), virtual-keyboard,
  virtual-pointer, pointer-constraints ve relative-pointer (oyunlar),
  pointer-gestures, tablet v2, dokunmatik ekran, touchpad jestleri.
- Çıktı ve pencere listesi: wlr-output-management (kanshi, wlr-randr),
  foreign-toplevel (wlr ve ext), drm-lease (VR), keyboard-shortcuts-inhibit,
  security-context, content-type, alpha-modifier, xdg-foreign, xdg-dialog,
  system-bell.
- İzin sistemi (`ecosystem` ve izin kuralları), bildirim katmanı, hyprcursor
  formatı.

Bitti sayılır: uyumluluk tablosundaki bütün protokoller "tam" durumunda.
OBS ile ekran paylaşımı ve fcitx5 ile yazı girişi çalışıyor.

## Aşama 8: Eklentiler

`hyprpm` eklentileri Hyprland'in C++ iç yapısına (ABI) bağlı, Rust'tan uyumlu
hale getirilemez. Seçenekler:

1. Eklenti desteği olmaz; popüler eklentilerin özellikleri (hyprexpo,
   hyprbars, hyprtrails vb.) doğrudan compositor'a eklenir.
2. Kararlı bir Rust eklenti API'si (dylib).
3. WASM tabanlı, sandbox içinde çalışan eklentiler.

Bu karar Aşama 3'ten sonra verilmeli.

## Aşama 9: Sağlamlaştırma ve yayın

- Çökme raporu ve kurtarma, uzun süreli kararlılık testleri.
- Config parser'ı ve IPC için fuzzing.
- Paketleme (AUR, Nix), kullanıcı dokümantasyonu, Hyprland'den geçiş
  rehberi.

## Riskler

- **Linux donanımı şart.** Aşama 0'dan sonraki her şey gerçek ekranda,
  klavyede ve GPU'da denenmeli. CI yalnızca derleme, mantık ve headless
  testleri yapabilir. Windows'tan geliştirme bu noktada darboğaz olur.
- **Hareketli hedef:** Hyprland her ay sürüm çıkarıyor. Sürüm sabitlenmezse
  uyumluluk hiç bitmez.
- **Smithay sınırları:** bazı protokoller (örneğin Hyprland'e özel olanlar,
  color-management) Smithay'de yok. Bunlar ya Smithay'e katkı olarak ya da
  bu projede yazılacak.
- **Shader işi:** Hyprland'in görünümünü (blur, köşeler, animasyonlar)
  piksel düzeyinde yakalamak zaman alır ve GPU'ya göre farklılık gösterir.
