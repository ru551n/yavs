-- difftest: top=psl_sequences_ops std=2008 ref=hand exit=1 ignore=path,order
-- ref=hand: the first reference simulator rejects or mishandles `|`, `within`, `&`, `[=n]` and
-- sequences that start with `[*0 ..]`; the expected output is the LRM result, which the second
-- reference simulator also gives (its output agrees). IEEE 1850 6.1.1.2, 6.1.2. A `[*n]` that
-- starts a sequence is `true[*n]`; a boolean stands for a sequence of one before `|->`/`|=>` and
-- with a repetition after `never` or `cover`.
library ieee;
use ieee.std_logic_1164.all;
entity psl_sequences_ops is end;
architecture t of psl_sequences_ops is
  signal clk : std_logic := '0';
  signal a, b, c : std_logic := '0';
  signal n : integer := 0;
  constant PA : string := "1101001011100110";
  constant PB : string := "0110100110010101";
  constant PC : string := "0011010101001011";
  default clock is rising_edge(clk);
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk) begin
    if rising_edge(clk) then
      n <= n + 1;
      if n < PA'length then
        a <= '1' when PA(n + 1) = '1' else '0';
        b <= '1' when PB(n + 1) = '1' else '0';
        c <= '1' when PC(n + 1) = '1' else '0';
      else
        a <= '0'; b <= '0'; c <= '0';
      end if;
    end if;
  end process;

  alt:    cover {{a = '1'; b = '1'} | {c = '1'}} report "alt";
  alt2:   cover {{a = '1'} | {b = '1'}; c = '1'} report "alt2";
  within1: cover {b = '1' within {a = '1'; c = '1'; a = '1'}} report "within";
  amp1:   cover {{a = '1'; b = '1'} & {c = '1'[*3]}} report "amp";
  eq1:    cover {a = '1'[=2]} report "eq";
  eq2:    cover {a = '1'[=2]; b = '1'} report "eq then";
  opt1:   cover {a = '1'; b = '1'[*0 to 1]; c = '1'} report "optional";
  rng1:   assert always {a = '1'} |=> {b = '1'[*1 to 2]; c = '1'} report "imp range";
  alt3:   assert always {a = '1'} |=> {{b = '1'} | {c = '1'}} report "imp alt";
  anon1:  cover {[*]; a = '1'; b = '1'} report "anonymous repeat";
  anon2:  cover {[*2]; a = '1'} report "anonymous count";
  bare1:  assert always a = '1' |=> b = '0' report "boolean left side";
  bare2:  assert never a = '1'[*2] report "never repeat";
  bare3:  assert never (a = '1')[->2] report "never goto";
  bare4:  cover a = '1'[*2] report "bare repeat";
  seqp:   assert always {{a = '1'; b = '1'} | {c = '1'; c = '1'}} |-> a = '1' report "lhs alt";
end;
