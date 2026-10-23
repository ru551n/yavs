library IEEE;
use IEEE.std_logic_1164.all;
use IEEE.std_logic_arith.all;

package body STD_LOGIC_SIGNED is


    function maximum(L, R: INTEGER) return INTEGER is
    begin
        if L > R then
            return L;
        else
            return R;
        end if;
    end;


    function "+"(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to plus
        constant length: INTEGER := maximum(L'length, R'length);
        variable result  : STD_LOGIC_VECTOR (length-1 downto 0);
    begin
        result  := SIGNED(L) + SIGNED(R); -- pragma label plus
        return   std_logic_vector(result);
    end;

    function "+"(L: STD_LOGIC_VECTOR; R: INTEGER) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to plus
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := SIGNED(L) + R; -- pragma label plus
        return   std_logic_vector(result);
    end;

    function "+"(L: INTEGER; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to plus
        variable result  : STD_LOGIC_VECTOR (R'range);
    begin
        result  := L + SIGNED(R); -- pragma label plus
        return   std_logic_vector(result);
    end;

    function "+"(L: STD_LOGIC_VECTOR; R: STD_LOGIC) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to plus
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := SIGNED(L) + R; -- pragma label plus
        return   std_logic_vector(result);
    end;

    function "+"(L: STD_LOGIC; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to plus
        variable result  : STD_LOGIC_VECTOR (R'range);
    begin
        result  := L + SIGNED(R); -- pragma label plus
        return   std_logic_vector(result);
    end;

    function "-"(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to minus
        constant length: INTEGER := maximum(L'length, R'length);
        variable result  : STD_LOGIC_VECTOR (length-1 downto 0);
    begin
        result  := SIGNED(L) - SIGNED(R); -- pragma label minus
        return   std_logic_vector(result);
    end;

    function "-"(L: STD_LOGIC_VECTOR; R: INTEGER) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to minus
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := SIGNED(L) - R; -- pragma label minus
        return   std_logic_vector(result);
    end;

    function "-"(L: INTEGER; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to minus
        variable result  : STD_LOGIC_VECTOR (R'range);
    begin
        result  := L - SIGNED(R); -- pragma label minus
        return   std_logic_vector(result);
    end;

    function "-"(L: STD_LOGIC_VECTOR; R: STD_LOGIC) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to minus
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := SIGNED(L) - R; -- pragma label minus
        return   std_logic_vector(result);
    end;

    function "-"(L: STD_LOGIC; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to minus
        variable result  : STD_LOGIC_VECTOR (R'range);
    begin
        result  := L - SIGNED(R); -- pragma label minus
        return   std_logic_vector(result);
    end;

    function "+"(L: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to plus
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := + SIGNED(L); -- pragma label plus
        return   std_logic_vector(result);
    end;

    function "-"(L: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to minus
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := - SIGNED(L); -- pragma label minus
        return   std_logic_vector(result);
    end;

    function "ABS"(L: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        variable result  : STD_LOGIC_VECTOR (L'range);
    begin
        result  := ABS( SIGNED(L));
        return   std_logic_vector(result);
    end;

    function "*"(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
        -- pragma label_applies_to mult
        constant length: INTEGER := maximum(L'length, R'length);
        variable result  : STD_LOGIC_VECTOR ((L'length+R'length-1) downto 0);
    begin
        result  := SIGNED(L) * SIGNED(R); -- pragma label mult
        return   std_logic_vector(result);
    end;
        
    function "<"(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to lt
        constant length: INTEGER := maximum(L'length, R'length);
    begin
        return   SIGNED(L) < SIGNED(R); -- pragma label lt
    end;

    function "<"(L: STD_LOGIC_VECTOR; R: INTEGER) return BOOLEAN is
            -- pragma label_applies_to lt
    begin
        return   SIGNED(L) < R; -- pragma label lt
    end;

    function "<"(L: INTEGER; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to lt
    begin
        return   L < SIGNED(R); -- pragma label lt
    end;

    function "<="(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to leq
    begin
        return   SIGNED(L) <= SIGNED(R); -- pragma label leq
    end;

    function "<="(L: STD_LOGIC_VECTOR; R: INTEGER) return BOOLEAN is
        -- pragma label_applies_to leq
    begin
        return   SIGNED(L) <= R; -- pragma label leq
    end;

    function "<="(L: INTEGER; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to leq
    begin
        return   L <= SIGNED(R); -- pragma label leq
    end;

    function ">"(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to gt
    begin
        return   SIGNED(L) > SIGNED(R); -- pragma label gt
    end;

    function ">"(L: STD_LOGIC_VECTOR; R: INTEGER) return BOOLEAN is
        -- pragma label_applies_to gt
    begin
        return   SIGNED(L) > R; -- pragma label gt
    end;

    function ">"(L: INTEGER; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to gt
    begin
        return   L > SIGNED(R); -- pragma label gt
    end;

    function ">="(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to geq
    begin
        return   SIGNED(L) >= SIGNED(R); -- pragma label geq
    end;

    function ">="(L: STD_LOGIC_VECTOR; R: INTEGER) return BOOLEAN is
        -- pragma label_applies_to geq
    begin
        return   SIGNED(L) >= R; -- pragma label geq
    end;

    function ">="(L: INTEGER; R: STD_LOGIC_VECTOR) return BOOLEAN is
        -- pragma label_applies_to geq
    begin
        return   L >= SIGNED(R); -- pragma label geq
    end;

    function "="(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return BOOLEAN is
    begin
        return   SIGNED(L) = SIGNED(R);
    end;

    function "="(L: STD_LOGIC_VECTOR; R: INTEGER) return BOOLEAN is
    begin
        return   SIGNED(L) = R;
    end;

    function "="(L: INTEGER; R: STD_LOGIC_VECTOR) return BOOLEAN is
    begin
        return   L = SIGNED(R);
    end;

    function "/="(L: STD_LOGIC_VECTOR; R: STD_LOGIC_VECTOR) return BOOLEAN is
    begin
        return   SIGNED(L) /= SIGNED(R);
    end;

    function "/="(L: STD_LOGIC_VECTOR; R: INTEGER) return BOOLEAN is
    begin
        return   SIGNED(L) /= R;
    end;

    function "/="(L: INTEGER; R: STD_LOGIC_VECTOR) return BOOLEAN is
    begin
        return   L /= SIGNED(R);
    end;

    function SHL(ARG:STD_LOGIC_VECTOR;COUNT: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is 
    begin
       return STD_LOGIC_VECTOR(SHL(SIGNED(ARG),UNSIGNED(COUNT)));
    end; 

    function SHR(ARG:STD_LOGIC_VECTOR;COUNT: STD_LOGIC_VECTOR) return STD_LOGIC_VECTOR is
    begin
       return STD_LOGIC_VECTOR(SHR(SIGNED(ARG),UNSIGNED(COUNT)));
    end;
 


--  This function converts std_logic_vector to a signed integer value
--  using a conversion function in std_logic_arith
    function CONV_INTEGER(ARG: STD_LOGIC_VECTOR) return INTEGER is
        variable result    : SIGNED(ARG'range);
    begin
        result    := SIGNED(ARG);
        return   CONV_INTEGER(result);
    end;
end STD_LOGIC_SIGNED;


