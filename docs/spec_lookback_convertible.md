# ルックバック・転換社債・MSCB の仕様案（P16、仮）

【2026-11-22。Stage 1〜4 は実装と照合が済んだ】

経路依存の商品を、「明日からの価値を計算するために、今日までの何を覚えておくか」（持ち越す**状態**）という一つの問いで並べ、契約の構造を使って状態の次元を落とす。対象は、ルックバック（Stage 1）、固定転換価額の転換社債（CB、Stage 2）、転換時にリセットする MSCB（Stage 3）、転換価額を持ち越す MSCB（Stage 4）である。

## 1. 状態の表

| 商品 | 今日までに覚えておくもの | 計算の次元 | 縮約の理由 |
|---|---|---|---|
| ヨーロピアン | なし（株価 $S$ だけ） | 1 | 価格は $S$ と $K$ の 1 次同次なので、比 $S/K$ で表せる |
| ルックバック（浮動ストライク） | 監視日の最小値 $m$（または最大値 $M$） | $(S, m)$ から $z = S/m$ の 1 次元 | 価格が $(S, m)$ について 1 次同次 |
| 固定転換価額の CB | なし（$S$ だけ。時間は別） | 1 | 転換価値 $\kappa S$ が $S$ の関数 |
| 転換時リセットの MSCB | なし（$S$ だけ） | 1 | 転換価値が、同時点の $S$ だけの関数（5 章） |
| 転換価額を持ち越す MSCB | 現在の転換価額 $K$ | $(S, K)$ の 2 次元。下限・上限がなければ $z = S/K$ の 1 次元 | $S/K$ の相似（6 章） |
| MBS、TARN（第 14 章） | 元本残高、累積クーポン、金利の履歴 | 縮約できない（パスの生成） | 経路ごとに状態を持ち越す |

## 2. 照合基準の確認（2026-11-22）

【実行確認。基準を作れることの確認で、rmql との一致はまだ調べていない】QuantLib R パッケージ 1.42 で、次のものが動いた。

| 対象 | R のクラス | 試した例の結果 |
|---|---|---|
| 浮動ストライクのルックバック・コール（連続監視、解析解） | `ContinuousFloatingLookbackOption`、`AnalyticContinuousFloatingLookbackEngine` | $S = 100$、これまでの最小値 100、$r = 3\%$、$q = 1\%$、$\sigma = 25\%$、1 年：19.07298 |
| 固定ストライクのルックバック・コール（連続監視、解析解） | `ContinuousFixedLookbackOption`、`AnalyticContinuousFixedLookbackEngine` | 同じ条件で $K = 100$、これまでの最大値 100：22.13615 |
| 転換社債（固定クーポン） | `ConvertibleFixedCouponBond`、`BinomialCRRConvertibleEngine`（ほかに EQP、J4、JR、LR、Tian、Trigeorgis） | アメリカン型の転換、5 年、年 2% クーポン、転換比率 1.0、信用スプレッド 2%、200 ステップ：114.1045 |

- ほかに、部分監視のルックバック（`...Partial...`）、`CallabilitySchedule`、`SoftCallability`、`DividendSchedule`、ゼロクーポンと変動利付の転換社債のクラスがある。
- **R のパッケージに、離散監視のルックバックのエンジンはない**。離散監視の基準は、連続監視への収束と、独立なモンテカルロで作る。
- 転換社債のエンジンの規約は、QuantLib v1.42 のソースを読んで確認した（4 章）。2 成分の分解ではなく、価値と転換確率を持つ格子である。

## 3. Stage 1：ルックバック【実装済み・実行照合済み（2026-11-22）】

実装は `crates/rmql/src/lookback.rs`、R の入口は `R/lookback.R`、照合用は `tools/golden/p16_lookback.R`（`tests/golden/option_lookback.csv`）。

**商品**：満期 $T$ までの監視日の株価の最小値 $m$、最大値 $M$（これまでに観測した値を、入力の `running_extremum` として与える）を使う。

| 種類 | コール | プット |
|---|---|---|
| 浮動ストライク | $S_T - m$ | $M - S_T$ |
| 固定ストライク | $\max(M - K, 0)$ | $\max(K - m, 0)$ |

`running_extremum` は、浮動コールと固定プットでは最小値（現物以下）、浮動プットと固定コールでは最大値（現物以上）で、反対側の値はエラー。

**連続監視**【実行照合済み】：閉じた式。標準の式（Goldman–Sosin–Gatto、Conze–Viswanathan）で、QuantLib のソースは写していない（記憶でも書いていない）。ドリフト $b = r - q$ で割る形なので、$|b| < 10^{-4}$ では括弧の部分 $G(b)/b$ を $b$ の 3 次までの展開に替える（$b = 0$ での $G'(0)$、$G''(0)$、$G'''(0)$ を導いた）。式の丸め誤差は $10^{-16}/|b|$、展開の誤差は $b^3$ の程度で、$10^{-4}$ で釣り合う。**QuantLib 1.42 は $b = 0$ ちょうどで NaN か無限大を返す**（$b$ で割るため。400 行の乱数入力のうち $r = q$ の 40 行がすべてそうだった）。rmql は極限の値を返す。

**固定ストライクの還元**：$\max(M, K) = S_T + (\max(M, K) - S_T)$ から、固定コール $=$ 浮動プット（最大値 $\max(M, K)$）$+\, S e^{-qT} - K e^{-rT}$、固定プット $=$ 浮動コール（最小値 $\min(m, K)$）$+\, K e^{-rT} - S e^{-qT}$。連続監視にも離散監視にも使い、固定の式を別に実装しない（QuantLib の固定ストライクの解析解との一致が、この還元の検証にもなる）。

**離散監視（相似縮約の木）**【実装済み】：

- 浮動ストライクの価格は $(S, m)$ について 1 次同次なので、$V = m\, w(z)$、$z = S/m$。$\ln z$ の格子 $z = u^j$（$u = e^{\sigma\sqrt{\Delta t}}$）を、**$z = 1$ に固定**してとる。確率と割引は `options.rs` の CRR 木と同じ。
- **監視日の更新（初版の案の誤りを直した）**：コールでは、価格が最小値を下回る（$z < 1$）と、その価格が新しい最小値になる。古い $m$ に対する価値は、$w_{\text{前}}(z) = z\, w_{\text{後}}(1)$（$z < 1$）、$w_{\text{前}}(z) = w_{\text{後}}(z)$（$z \ge 1$）。初版の案の「$j \to \max(j, 0)$」は、$z$ を掛ける項が抜けていた。プットは対称（$z > 1$ で同じ形）。
- 初期の $z_0 = S/m$ は、格子の点とは限らないので、時刻 0 の 2 点の値を $z$ で線形に補間する。
- 監視時刻は最も近い段に置く。最終の払い戻しは $\max(\cdot, 0)$ で、満期が監視日でないときに効く。監視日がなければ、極値は動かず、行使価格 $m$ のヨーロピアンになる。
- グリークス：`gamma` は、時刻 0 の格子の点の値の 2 階差分（木の値は現物について区分的に線形）。ほかは木の自動微分。

**公開 API**（`options.rs` の流儀：`Operand` の総称型、グリークスは自動微分）：

| | Rust | R |
|---|---|---|
| 連続監視 | `compute_lookback_value<T: Operand>`、`compute_lookback_greeks`（`LookbackStrike`、`OptionType`、`LookbackInputs`） | `compute_lookback_option_value(lookback_type, option_type, spot, running_extremum, strike, rate, dividend_yield, volatility, time, on_error)` |
| 離散監視 | `compute_discrete_lookback_value<T: Operand>`、`compute_discrete_lookback_greeks`（`monitoring: &[f64]`、`steps`） | `compute_discrete_lookback_option_value(..., monitoring_times, steps = 400L, on_error)` |

戻り値は、価値、`delta`、`gamma`、`vega`、`rho`、`dividend_rho`、`theta`、`strike_sensitivity`、`extremum_sensitivity`。浮動ストライクの `strike` は `NA`（現物が代わりに入る）。

**検査結果**：

| 検査 | 基準 | 結果 |
|---|---|---|
| 連続監視：浮動・固定、コール・プット、400 行（$b = \pm 10^{-5}$ の 40 行を含む） | QuantLib 1.42 の解析解 | 相対 1e-11 または絶対 1e-11 以内。最大の相対差は 2.9e-11（`tests/golden_lookback.rs`） |
| 離散監視：全ステップで監視、400 / 1600 / 6400 段 | 連続監視の解析解 | 誤差が段数の 4 倍ごとに約半分（比 0.5 ± 0.03、コールとプット）。$1/\sqrt{\text{段数}}$ での外挿が、解析解に相対 1e-3 以内 |
| 離散監視：月次 12 回、100・200 段 / 月 | 独立なモンテカルロ（厳密な対数正規、対称変数、20 万組） | 標準誤差の 4 倍 + 0.2% 以内（コールとプット） |
| 性質 | 同次性（現物と極値を同じ比で動かすと価格が同じ比）、固定と浮動の還元、監視日なしはヨーロピアン、ドリフト 0 の連続性、$b$ の展開と式の一致 | 合格 |
| グリークス | 自動微分と中心差分 | 連続：合格。木：デルタとベガは一致、ガンマは連続監視の値の 30% 以内 |
| 入力の検査 | 極値の側の誤り、時刻の順序と範囲、非正の値、2 段未満の木 | エラー |

**未了・制約**：連続性の補正（Broadie–Glasserman–Kou 型、文献の内容は未確認）は入れていない。木の誤差は $1/\sqrt{\text{段数}}$ で遅い（段数 6400 で連続監視との差が、コールで約 0.65%、プットで約 0.9%）。離散監視の木の速さは、まだ測っていない。

## 4. Stage 2：固定転換価額の CB【実装済み・実行照合済み（2026-11-22）】

実装は `crates/rmql/src/convertible.rs`、R の入口は `R/convertible.R`（`compute_convertible_bond_value`）、照合用は `tools/golden/p16_convertible.R`（`tests/golden/convertible_bonds.csv`、300 行）。方法と規約は、QuantLib v1.42（`41b0e146`）のソース（`binomialconvertibleengine.hpp`、`discretizedconvertible.{hpp,cpp}`、`tflattice.hpp`、`convertiblebonds.cpp`）を読んで写した（著作権表示は `THIRD_PARTY_NOTICES`）。

**初版の案の修正**：QuantLib の TF の格子は、株式の部分と現金の部分の 2 成分に分ける形ではない。節点ごとに、債券全体の価値 $V$ と、転換に至る確率 $p$ の 2 本を持つ。

**格子と割引**【ソースで確認】：

- Cox–Ross–Rubinstein の木（`options.rs` の `crr_tree` と同じ確率 $p_u = \tfrac12 + \tfrac12 \cdot \text{ドリフト} / dx$）。時間は、決済日から最後の行使日までを一様に `steps` 段に分ける。木の定数は、一定の $r$、$q$、$\sigma$。
- 節点の割引率は、$p\,r + (1 - p)(r + s)$（$s$ は信用スプレッド）。1 段戻る式は、**単純割引** $V_j \leftarrow p_d V_j / (1 + \text{率}_j \Delta t) + p_u V_{j+1} / (1 + \text{率}_{j+1} \Delta t)$（指数ではない。率は 1 段先の節点のもの）。$p$ も同じ重みで戻す。
- 各節点の時刻での調整は、**コール・プット → クーポン → 転換**の順。転換は、$V \le \kappa S$ なら $V = \kappa S$、$p = 1$。新しい $p$ から割引率を作るのは、その時刻の調整の**前**なので、その時刻の転換は、その時刻の率の配列に反映されない（次の 1 段が使う率は、調整前のもの）。最終の時刻だけは、調整のあとに率を作る。
- 満期の初期値は償還額。満期のクーポンは、償還額に足してから、転換を判定する。
- クーポン、コール、行使の日付は、最も近い格子点に置く（同じ距離なら早い点）。クーポンは決済日**より後**のものだけを使う。コールが決済日以前なら捨てる。木の終わりは、最後の行使日（債券の満期より前でもよく、それより後のクーポンは最後の点に置く）。
- コール：ソフトコール（`trigger`）なら、株価が `償還額 / 転換比率 × trigger` 以上の節点で、$V = \min(\max(\text{価格}, \kappa S), V)$。トリガーなしで、その時刻に転換できるなら、同じ式を全節点で。転換できない時刻は $V = \min(\text{価格}, V)$。プットは $V = \max(V, \text{価格})$。クリーン価格のコールは、コール日の経過利子を足して、ダーティにする。

**入力**：`ConvertibleTerms`（償還額、`ConversionTerms::Fixed { ratio }`、`ConversionExercise`（ヨーロピアン・アメリカン・バミューダン）、クーポン（時刻と額）、`Callability`（時刻、種類、価格、トリガー））と、`ConvertibleInputs`（現物、$r$、$q$、$\sigma$、信用スプレッド）。時刻は決済日からの年数。債券の条件と日付からは、`create_convertible_terms(bond, settlement, day_count, ...)` が作る。**対象外**（初期版）：離散の配当、金利の確率過程、CRR 以外の木。QuantLib と違い、段数は 2 以上が要る。

**公開 API**：

| | Rust | R |
|---|---|---|
| 価値 | `compute_convertible_value<T: Operand>(&ConvertibleTerms, &ConvertibleInputs<T>, steps)` | `compute_convertible_bond_value(bond, settlement_date, exercise, exercise_dates, conversion_ratio, spot, rate, dividend_yield, volatility, credit_spread, steps, callabilities, day_count, on_error)` |
| グリークス | `compute_convertible_greeks`（`delta`、`gamma`、`vega`、`rho`、`dividend_rho`、`credit_spread_sensitivity`） | 同じ関数の戻り値の列 |

`delta` と `gamma` は、格子の最初の 1・2 段の値から（Hull の式、`options.rs` の木と同じ）。ほかは自動微分。QuantLib は価値だけを返す。現物を格子の間隔より細かくずらした差分は、転換とコールが節点で切り替わるため、ばらつく（実測：コールつき・アメリカン、段数 400 で、ずらし幅 0.05〜2 のデルタが 0.74〜0.75、最初の段の差分は 0.797、幅 5 の差分は 0.799）。

**速さ**【実測。2026-11-22、R の windows-gnu ビルド、アメリカン、コールとプットつき、価値とグリークスを一緒に。この PC での値】：100 段 0.02 秒、300 段 0.08 秒、1000 段 0.8 秒、3000 段 10〜19 秒。段数の 2 乗で増える（自動微分の記録が段数の 2 乗の大きさ）。グリークスなしの価値だけの速さは、まだ測っていない。

**QuantLib との違い（浮動小数点の端数）**【実行照合済み】：QuantLib の `TimeGrid(end, steps)` は、点を $\Delta t \times i$（$\Delta t = \text{end}/\text{steps}$）で作るので、最後の点が `end` より 1 ulp 小さいことがある。アメリカン型の転換の判定は、最後の時刻で `time() <= 最後の行使日の格子点` と書かれているため、その場合に**満期での転換が許されない**。段数によって起きる（300 行のうち 9 行、すべてアメリカンで段数 50 か 100）。rmql の公開関数は、満期でも転換できる。QuantLib の挙動は、`compute_convertible_value_like_quantlib`（ドキュメント非表示）で再現し、照合に使う。端数の影響を受ける行では、2 つの値は、**どちらの向きにも**、最大 0.67% 違う：満期で転換できると、その節点の転換確率が 1 でなくなり、前の段の割引率（転換確率で混ぜる）が上がって、値が下がることがある（この格子の値は、転換する権利について単調ではない）。

**検査結果**：

| 検査 | 基準 | 結果 |
|---|---|---|
| 300 行：ヨーロピアン 107、アメリカン 90、バミューダン 103。コール・プット・ソフトコール・クリーン価格のコール、段数 20〜300 | QuantLib 1.42（`ConvertibleFixedCouponBond`、`BinomialCRRConvertibleEngine`） | 端数を再現した版は、300 行すべてで相対 7.8e-12 以内。公開関数は、端数の影響のない 291 行で 7.8e-12 以内、影響のある 9 行で最大 0.67%（`tests/golden_convertible.rs`） |
| R の入口：債券から項を作る、クリーン価格の経過利子、バミューダン | QuantLib R の 3 つの値 | 相対 1e-9 以内（`tests/testthat/test-convertible.R`） |
| 極限：転換比率が極小 | 割引率 $r + s$ の単純割引の和（厳密な式） | 相対 1e-10 以内 |
| 極限：信用スプレッド 0、クーポンなし、ヨーロピアン、コールなし | 閉じた式 $F e^{-rT} + \kappa\, \mathrm{call}(S, F/\kappa)$（2000 段） | 相対 3e-3 以内（木と単純割引の誤差） |
| 性質 | 現物について増加、信用スプレッドについて減少、転換権の価値は非負、コールは価値を下げ、プットは上げ、ソフトコールはコールより弱い | 合格 |
| グリークス | ベガ・ロー・信用スプレッド感応度は中心差分と一致、デルタは最初の段の差分と幅 5 の差分が一致 | 合格 |

## 5. Stage 3：転換時リセットの MSCB（1 次元）【実装済み・実行照合済み（2026-11-22）】

実装は `crates/rmql/src/convertible.rs`（`ConversionTerms::Reset`）、R の入口は `R/convertible.R`（`compute_reset_convertible_bond_value`）。Stage 2 の格子をそのまま使い、転換価値の関数だけを差し替える。

**契約**【同時点の株価を参照する】：転換するとき、転換価額を $K(S) = \min\bigl(K_{\max},\ \max(K_{\min},\ \alpha S)\bigr)$（$K_{\min} \le K_{\max}$、$\alpha$ は例えば 0.9）とし、額面 $F$ に対して $F/K(S)$ 株を受け取る。転換で受け取る株式の価値は、

$$
\frac{F\,S}{K(S)} = F\,\max\!\left(\frac{S}{K_{\max}},\ \min\!\left(\frac{S}{K_{\min}},\ \frac1\alpha\right)\right)
$$

で、$S$ だけの関数（上限がなければ $F\min(S/K_{\min},\, 1/\alpha)$。$K_{\max}$ は `Inf` でよい）。持ち越す状態がないので、Stage 2 と同じ 1 次元の格子で解ける。額面 $F$ は、債券の決済日の元本。

**実装の形**：

```text
ConversionTerms::Fixed { ratio }                          // 転換価値 ratio * S
ConversionTerms::Reset { face, alpha, k_min, k_max }      // 転換価値 face * S / K(S)
```

**ソフトコールは使えない**：ソフトコールのトリガーは、償還額と転換価値が等しくなる株価（`償還額 / 転換比率`）を基準にするが、リセット型には固定の転換比率がない。トリガーつきのコールは、リセット型ではエラーにする。

**性質**：$\alpha < 1$ で同時点参照なら、$S \ge K_{\min}/\alpha$ のとき、転換価値は $F/\alpha > F$ で頭打ちになる。アメリカン型の転換なら、保有者は発行直後でも、その株価以上で**即時に転換して $F/\alpha$ を得る**（値が現物に依存しなくなり、グリークスは 0）。同時点参照の条項そのものの性質で、実装の誤りではない。実際の契約が、この性質をどう抑えているか（転換できる日の制限、参照する株価の定め方など）は、初期版の対象外で、確認していない（7 章）。

**検査結果**：

| 検査 | 基準 | 結果 |
|---|---|---|
| 転換価値の形：$K$ が下限・割引・上限の各区間 | $F S / K(S)$ を直接計算 | 相対 1e-12 以内 |
| 極限：$K_{\min} = K_{\max} = K$（$\alpha$ は 0.5、0.9、1.0、1.3） | 転換比率 $F/K$ の固定 CB（コール・プットつき、アメリカン、250 段） | 相対 1e-10 以内（Rust と R） |
| ヨーロピアン、信用スプレッド 0、クーポンなし | 閉じた式（下記）、2500 段 | 相対 3e-3 以内（$\alpha$ = 0.9・0.8・0.95、上限あり・なし）。R では 1500 段で 5e-3 以内 |
| 入力の検査 | 下限が上限を超える、$\alpha$・下限・額面が正でない、上限が NaN、ソフトコール | エラー |
| グリークス | 現物が $K_{\min}/\alpha$ より下で、有限、デルタ・ベガが非負、信用スプレッド感応度が負 | 合格 |

**閉じた式**：転換価値を $F\,g(S)$、$g(S) = \max\bigl(S/K_{\max},\ \min(S/K_{\min},\ 1/\alpha)\bigr)$ と書くと、$g(S) = S/K_{\min} - (S/K_{\min} - 1/\alpha)^+ + (S/K_{\max} - 1/\alpha)^+$（$K_{\min} \le K_{\max}$、$\alpha \le 1$）と分解できるので、ヨーロピアンの転換（満期 $T$、クーポンなし、信用スプレッドなし、コールなし）の価値は、ブラック・ショールズのコールの価格 $\mathrm{call}(K)$ で

$$
V = F e^{-rT} + F\left[\frac{\mathrm{call}(K_{\min})}{K_{\min}} - \frac{\mathrm{call}(K_{\min}/\alpha)}{K_{\min}} + \frac{\mathrm{call}(K_{\max}/\alpha)}{K_{\max}}\right]
$$

となる（償還額 $F$ と、転換した場合の $F\,g(S_T)$ の大きいほう。$g > 1$ の部分だけが転換）。この式は、格子の実装とは独立な基準として使った。

## 6. Stage 4：転換価額を持ち越す MSCB【実装済み・実行照合済み（2026-11-22）】

実装は `crates/rmql/src/mscb.rs`（Stage 2 の格子を `Engine` として 1 段ずつ呼ぶ形に分けて、再利用した）、R の入口は `R/convertible.R`（`compute_carried_reset_convertible_bond_value`）。

**契約**：リセットの日付 $\tau_1 < \dots < \tau_n$ に、転換価額 $K$ を、同じ時点の株価 $S$ で更新し、次のリセットまで持ち越す。**リセットは、その日の転換の前に行う**（その日の転換は、新しい $K$ を使う）。転換するとき、受け取る株式の価値は $F S / K$。

```text
ResetRule::Both         { alpha, k_min, k_max }   // K+ = min(k_max, max(k_min, alpha * S))。直前の K- に依らない
ResetRule::DownwardOnly { alpha, k_min }          // K+ = min(K-, max(k_min, alpha * S))
```

**2 次元の格子（`MscbMethod::Grid { price_nodes }`）**：

- 転換価額の格子の点ごとに、転換比率 $F/K$ の Stage 2 の格子（現物の木）を持つ。リセットの間は、点どうしが独立に戻る。ソフトコールは、各点の転換比率で、そのまま使える。
- リセット日には、各現物の節点で、新しい価格 $K^+(S, K^-)$ のところの値を、$K$ 軸で補間する。補間するのは、価値、転換確率、割引率の 3 本（TF の格子の状態）。**補間は $1/K$ について線形**（転換価値が $1/K$ に線形なので、転換価値の部分で誤差が出ない）。
- 格子の範囲：初期価格を点に含め、最後のリセット時点の現物の分布の **±6 標準偏差**（木の範囲より狭いときだけ）で、リセットが与える最低・最高の価格まで、$\ln K$ に等間隔の `price_nodes` 個。範囲の外では、端の点の値に止める（確率は 1e-8 より小さい）。初版は、木の端までを範囲にしたため、段数を増やすと間隔が粗くなって精度が落ちた（200 段で +7e-4、400 段で +4e-3）。
- グリークス：格子の価格を固定して、自動微分（`delta`、`gamma` は初期価格の点の木の最初の段から）。

**1 次元の縮約（`MscbMethod::Scaled`）**：$K_{\min} = 0$（`Both` は $K_{\max}$ も無限大）で、絶対的な水準が入らないとき、価値は $z = S/K$ だけの関数。現物 $S_0/K_0$、転換比率 $F$ の Stage 2 の木と、リセット日の写像で解く。写像は、`DownwardOnly` が $z^+ = \max(z^-, 1/\alpha)$、`Both` が $z^+ = 1/\alpha$（初版の案のとおり）。木の節点の間にある $1/\alpha$ の値は、$z$ について線形に補間する。下限か上限があると、絶対的な水準が入るので、エラーにする（2 次元を使う）。

**公開 API**：

| | Rust | R |
|---|---|---|
| 価値 | `compute_mscb_value<T: Operand>(&MscbTerms, &ConvertibleInputs<T>, steps, MscbMethod)` | `compute_carried_reset_convertible_bond_value(bond, settlement_date, exercise, exercise_dates, reset_dates, rule, conversion_price_ratio, initial_conversion_price, lower_conversion_price, upper_conversion_price, spot, ..., steps, method, price_nodes, greeks, callabilities, ...)` |
| グリークス | `compute_mscb_greeks` | 同じ関数（`greeks = FALSE` で値だけ。格子では、桁違いに速い） |

**検査結果**：

| 検査 | 基準 | 結果 |
|---|---|---|
| リセットなし | Stage 2 の固定 CB（転換比率 $F/K_0$、コール・ソフトコール・プットつき、アメリカン） | 相対 1e-10 以内（格子・縮約とも、Rust と R） |
| 価格を変えないリセット：`DownwardOnly` で $\alpha$ が大きい（3 回のリセット）、`Both` で $K_{\min} = K_{\max} = K_0$ | 同じ固定 CB | 相対 1e-10 以内（リセットの写像が恒等になる） |
| ヨーロピアンの転換、リセット 1 回、クーポン・スプレッド・コールなし、`Both` | **閉じた式** $F e^{-rT} + e^{-r\tau} (F/\alpha)\, \mathrm{call}(1, \alpha, T - \tau)$（リセット後の転換価値が $(F/\alpha) S_T / S_\tau$ になる） | 縮約：相対 3e-3 以内（2000 段）。格子：1e-2 以内（400 段、120 点）。R でも確認 |
| 同じ条件、`DownwardOnly`（$\alpha$ = 0.9・0.7） | $S_\tau$ の分布での数値積分 $F e^{-rT} + e^{-r\tau} E[(F/K^+)\, \mathrm{call}(S_\tau, K^+, T - \tau)]$、$K^+ = \min(K_0, \alpha S_\tau)$（シンプソン則、4000 区間） | 縮約：3e-3 以内、格子：1e-2 以内 |
| 格子と縮約（`DownwardOnly`、4 回のリセット、コールつき、アメリカン、300 段） | 互いに | 点数 20・40・80・160 で、相対差 3.8e-3、5e-5、9e-4、1.0e-3。点を増やすと、格子の補間の誤差は減り、残る 1e-3 程度は、縮約の $z$ の補間と、2 つの補間の違い（未確認の推定） |
| 入力の検査 | リセット時刻の範囲・順序・同じ段、価格・額面・$\alpha$ が正でない、上限が下限より小さい、点数 1、縮約に限度がある | エラー |

**速さ**【実測。2026-11-22、R の windows-gnu ビルド、アメリカン、コールつき、4 回のリセット、`DownwardOnly`。他の処理と同時に実行したので、ばらつきがある】：

| 段数 | 縮約（グリークスつき） | 格子 80 点（値だけ） | 格子 80 点（グリークスつき） |
|---|---|---|---|
| 200 | 0.03 秒 | 0.07 秒 | 2.8 秒 |
| 400 | 0.15 秒 | 0.31 秒 | 30 秒 |

縮約の値だけは、測定の分解能（10 ms）以下。格子は、値だけなら点数に比例する。グリークスつきは、自動微分の記録が、点数と段数の 2 乗に比例して大きくなり、遅い（実測では、点数を 2 倍にして 5 倍：メモリの増加の影響と推定、未確認）。**縮約は、格子（80 点）の約 100 倍速い**（値だけどうしの比は、段数 800 で約 100 倍）。限度のない契約では、縮約を使うのが勧め。

## 7. 初期版で扱わない実務の条項

転換請求の直前取引日の終値の参照、希薄化（新株発行などによる転換価額の調整）、月間の行使上限、離散の配当、金利の確率過程、転換時リセット（Stage 3）でのソフトコール（固定の転換比率がないため。Stage 2 と Stage 4 では使える）。モデルの検証と、実務の条項の差を分けるため、これらは初期版の対象外とし、ここに明記する。契約の文面と数理モデルの差は、利用者が判断する。

## 8. 計画

| 順 | 内容 | 成果物 |
|---|---|---|
| 0 | この文書の確認 | 修正した本書 |
| 1 | Stage 1：連続監視（解析解）、離散監視（相似縮約の木）、グリークス【済み】 | `crates/rmql/src/lookback.rs`、R `R/lookback.R`、`tools/golden/p16_lookback.R` |
| 2 | Stage 2：固定 CB【済み】 | `convertible.rs`、`R/convertible.R`、`tools/golden/p16_convertible.R` |
| 3 | Stage 3：`ConversionTerms::Reset`【済み】 | Stage 2 の拡張（`convertible.rs`、`R/convertible.R`） |
| 4 | Stage 4：2 次元と 1 次元の縮約、速さの比【済み】 | `mscb.rs`、`R/convertible.R` |

各段階：照合用スクリプト → Rust → 照合テスト → 本書の更新 → 全テストと clippy → 報告（CLAUDE.md の段階の手順）。照合の許容誤差は、5.1 の表を初期案とし、実測を見て決める。既存の回帰テストの許容誤差は変えない。

## 9. 未確認と判断してほしい点

1. （済）QuantLib の転換社債エンジンの規約は、ソースを読んで確定した（4 章）。
2. 離散監視の木で、連続性の補正（Broadie–Glasserman–Kou 型）を入れるか（監視日は最も近い段に置く形で実装済み）。
3. （済）Stage 4 の $K$ 軸の補間は、$1/K$ について線形にした。3 次にする必要があるかは、実測（6 章）では未確認。
4. 公開関数の名前（`compute_lookback_option_value`、`compute_convertible_bond_value` など）と、段階の番号（P16）は仮である。
5. クライアント（Carmine の第 13 章）が呼ぶ引数の形。Carmine 側の章は、この計算エンジンが揃ってから書く。

## 10. Carmine の章との対応（参考）

Carmine Studio の第 13 章では、ヨーロピアン（$S/K$ で表せる）からルックバック、CB、MSCB へ進み、第 14 章（MBS、TARN）で経路ごとに状態を持ち越す計算、第 15 章（自動微分）で同じ PRDC の感応度へ進む案である。第 11 章の最後でも、答えの分かっているヨーロピアンの価格関数を学習する題材を置く案がある。章の構成は Carmine の側の決めごとで、rmql はこの仕様の計算を提供する。
