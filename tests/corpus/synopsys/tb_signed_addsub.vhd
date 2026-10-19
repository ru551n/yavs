-- difftest: top=tb_signed_addsub
library ieee;
use ieee.std_logic_1164.all;
library ieee;
use ieee.std_logic_arith.all;
use ieee.std_logic_signed.all;
entity tb_signed_addsub is end entity;
architecture t of tb_signed_addsub is
  procedure pr(constant s : in string) is begin report s; end procedure;
  type int_arr is array (natural range <>) of integer;
  constant INTS  : int_arr := (0, 1, -1, 2, -2, 3, 7, 8, -8, 100, 127, 128, -128, -129, 255, 256, -256, 1000, -1000, 32767, -32768, 65535, 1048581, 16777216, -16777217);
  constant SIZES : int_arr := (1, 2, 3, 7, 8, 9, 31, 32, 33, 40);
  constant WS    : int_arr := (1,2,7,8,31,32,33);
  constant WSC   : int_arr := (1, 2, 7, 8, 31);
  constant WSS   : int_arr := (1, 2, 7, 8, 31, 32);
  constant WSH   : int_arr := (1, 2, 3, 4);
  function ch(c : std_ulogic) return character is begin
    case c is when 'U' => return 'U'; when 'X' => return 'X'; when '0' => return '0'; when '1' => return '1';
      when 'Z' => return 'Z'; when 'W' => return 'W'; when 'L' => return 'L'; when 'H' => return 'H'; when '-' => return '-'; end case;
  end function;
  function elems(v : std_logic_vector) return string is
    variable r : string(1 to v'length);
    variable i : natural := 0;
  begin
    for p in v'range loop i := i + 1; r(i) := ch(v(p)); end loop;
    return r;
  end function;
  function img(v : std_logic_vector) return string is
  begin
    return integer'image(v'left) & "|" & integer'image(v'right) & "|" & boolean'image(v'ascending) & "|" & elems(v);
  end function;
  function rnd(seed, i : integer) return integer is
    variable h : integer;
  begin
    h := (seed * 7919 + i * 104729 + 12345) mod 46337;
    h := (h * h + 17) mod 46337;
    h := (h * 31 + seed) mod 46337;
    return h;
  end function;
  function fill(w, k : natural) return std_logic_vector is
    variable s : std_logic_vector(0 to w - 1);
    variable b : integer;
  begin
    for i in 0 to w - 1 loop
      b := rnd(k + 3 * w + 1, i) mod 2;
      case k mod 16 is
        when 0 => s(i) := '0';
        when 1 => s(i) := '1';
        when 2 => if i mod 2 = 0 then s(i) := '1'; else s(i) := '0'; end if;
        when 3 => if i mod 2 = 0 then s(i) := '0'; else s(i) := '1'; end if;
        when 4 => if i = 0 then s(i) := '1'; else s(i) := '0'; end if;
        when 5 => if i = w - 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 6 | 7 => if b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 8 => if i = (w / 2) then s(i) := 'X'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 9 => if i = (w - 1) then s(i) := 'U'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 10 => if i = 0 then s(i) := 'Z'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 11 => if i = (w / 3) then s(i) := 'W'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 12 => if i = (w - 1) then s(i) := '-'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
        when 13 => if b = 1 then s(i) := 'H'; else s(i) := 'L'; end if;
        when 14 => s(i) := 'X';
        when others => if i = 0 then s(i) := 'H'; elsif b = 1 then s(i) := '1'; else s(i) := '0'; end if;
      end case;
    end loop;
    return s;
  end function;
  function mkv(w, k, r : natural) return std_logic_vector is
    variable s : std_logic_vector(0 to w - 1) := fill(w, k);
    variable a : std_logic_vector(w - 1 downto 0);
    variable b : std_logic_vector(w + 2 downto 3);
    variable c : std_logic_vector(3 to w + 2);
  begin
    case r mod 4 is
      when 0 => a := s; return a;
      when 1 => return s;
      when 2 => b := s; return b;
      when others => c := s; return c;
    end case;
  end function;
  function mku(w, k, r : natural) return unsigned is begin return unsigned(mkv(w, k, r)); end function;
  function mks(w, k, r : natural) return signed is begin return signed(mkv(w, k, r)); end function;
  function ul(k : natural) return std_ulogic is
    constant t : std_ulogic_vector(0 to 8) := "UX01ZWLH-";
  begin return t(k); end function;
begin
  process
    variable kb, ii : integer;
  begin
    -- +[STD_LOGIC_VECTOR,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR]
    for wa in WS'range loop for wb in WS'range loop for j in 0 to 15 loop
      kb := (j * 7 + wa * 3 + wb) mod 16;
      pr("+[STD_LOGIC_VECTOR,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR] " & img(std_logic_vector(mkv(WS(wa),j,j+wa))) & " , " & img(std_logic_vector(mkv(WS(wb),kb,kb+wb+1))) & " => " & img(std_logic_vector'(mkv(WS(wa),j,j+wa) + mkv(WS(wb),kb,kb+wb+1))));
    end loop; end loop; end loop;
    -- +[STD_LOGIC_VECTOR,INTEGER->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 2 loop
      ii := (j * 3 + t * 5 + wi) mod 25;
      pr("+[STD_LOGIC_VECTOR,INTEGER->STD_LOGIC_VECTOR] " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " , " & integer'image(INTS(ii)) & " => " & img(std_logic_vector'(mkv(WS(wi),j,j+wi) + INTS(ii))));
    end loop; end loop; end loop;
    -- +[INTEGER,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 2 loop
      ii := (j * 3 + t * 5 + wi) mod 25;
      pr("+[INTEGER,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR] " & integer'image(INTS(ii)) & " , " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " => " & img(std_logic_vector'(INTS(ii) + mkv(WS(wi),j,j+wi))));
    end loop; end loop; end loop;
    -- +[STD_LOGIC_VECTOR,STD_ULOGIC->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 8 loop
      pr("+[STD_LOGIC_VECTOR,STD_ULOGIC->STD_LOGIC_VECTOR] " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " , " & ch(ul(t)) & " => " & img(std_logic_vector'(mkv(WS(wi),j,j+wi) + ul(t))));
    end loop; end loop; end loop;
    -- +[STD_ULOGIC,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 8 loop
      pr("+[STD_ULOGIC,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR] " & ch(ul(t)) & " , " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " => " & img(std_logic_vector'(ul(t) + mkv(WS(wi),j,j+wi))));
    end loop; end loop; end loop;
    -- -[STD_LOGIC_VECTOR,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR]
    for wa in WS'range loop for wb in WS'range loop for j in 0 to 15 loop
      kb := (j * 7 + wa * 3 + wb) mod 16;
      pr("-[STD_LOGIC_VECTOR,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR] " & img(std_logic_vector(mkv(WS(wa),j,j+wa))) & " , " & img(std_logic_vector(mkv(WS(wb),kb,kb+wb+1))) & " => " & img(std_logic_vector'(mkv(WS(wa),j,j+wa) - mkv(WS(wb),kb,kb+wb+1))));
    end loop; end loop; end loop;
    -- -[STD_LOGIC_VECTOR,INTEGER->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 2 loop
      ii := (j * 3 + t * 5 + wi) mod 25;
      pr("-[STD_LOGIC_VECTOR,INTEGER->STD_LOGIC_VECTOR] " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " , " & integer'image(INTS(ii)) & " => " & img(std_logic_vector'(mkv(WS(wi),j,j+wi) - INTS(ii))));
    end loop; end loop; end loop;
    -- -[INTEGER,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 2 loop
      ii := (j * 3 + t * 5 + wi) mod 25;
      pr("-[INTEGER,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR] " & integer'image(INTS(ii)) & " , " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " => " & img(std_logic_vector'(INTS(ii) - mkv(WS(wi),j,j+wi))));
    end loop; end loop; end loop;
    -- -[STD_LOGIC_VECTOR,STD_ULOGIC->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 8 loop
      pr("-[STD_LOGIC_VECTOR,STD_ULOGIC->STD_LOGIC_VECTOR] " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " , " & ch(ul(t)) & " => " & img(std_logic_vector'(mkv(WS(wi),j,j+wi) - ul(t))));
    end loop; end loop; end loop;
    -- -[STD_ULOGIC,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR]
    for wi in WS'range loop for j in 0 to 15 loop for t in 0 to 8 loop
      pr("-[STD_ULOGIC,STD_LOGIC_VECTOR->STD_LOGIC_VECTOR] " & ch(ul(t)) & " , " & img(std_logic_vector(mkv(WS(wi),j,j+wi))) & " => " & img(std_logic_vector'(ul(t) - mkv(WS(wi),j,j+wi))));
    end loop; end loop; end loop;
    pr("DONE");
    wait;
  end process;
end architecture;
