-- difftest: top=port_expr_record std=2008 stop=10ns
-- Non-static record expression actuals of in ports (LRM 6.5.6.3, #74): the equivalent
-- process `formal <= expr` drives every leaf of the formal, one delta after the expression
-- changes. A whole record formal and a record element of an array of records in an
-- individual association (a whole array of records: port_expr_record_array).
package port_expr_record_pkg is
  type pair is record
    a : integer;
    b : bit_vector(1 downto 0);
  end record;
  type pairs is array (natural range <>) of pair;
end package;
use work.port_expr_record_pkg.all;
entity port_expr_record_mon is
  generic (tag : string);
  port (p : in pair := (-1, "00"); ps : in pairs(0 to 1) := (others => (-2, "11")));
end;
architecture a of port_expr_record_mon is
begin
  process (p, ps) begin
    report tag & " p=" & integer'image(p.a) & "," & to_string(p.b)
      & " ps0=" & integer'image(ps(0).a) & "," & to_string(ps(0).b)
      & " ps1=" & integer'image(ps(1).a) & "," & to_string(ps(1).b) & " @" & time'image(now);
  end process;
end;
use work.port_expr_record_pkg.all;
entity port_expr_record is end;
architecture a of port_expr_record is
  signal x : integer := 1;
  signal v : bit_vector(1 downto 0) := "01";
  signal r : pair := (5, "10");
begin
  m1 : entity work.port_expr_record_mon generic map ("m1") port map (p => (x, v));
  m2 : entity work.port_expr_record_mon generic map ("m2")
    port map (p => r, ps(0) => (x * 2, not v), ps(1) => r);
  process begin
    wait for 1 ns;
    x <= 3;
    wait for 1 ns;
    v <= "11";
    r <= (7, "00");
    wait;
  end process;
end;
