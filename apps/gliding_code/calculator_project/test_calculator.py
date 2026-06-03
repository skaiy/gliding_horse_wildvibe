#!/usr/bin/env python3
"""
计算器程序 - 测试文件

对 Calculator 类进行全面的单元测试，覆盖正常运算、边界情况和异常场景。
"""

import pytest
import math
from calculator import Calculator, CalculatorError


class TestCalculator:
    """Calculator 类的单元测试。"""

    def setup_method(self):
        """每个测试方法执行前初始化计算器实例。"""
        self.calc = Calculator()

    # ========== 加法测试 ==========

    def test_add_two_positive_numbers(self):
        """测试两个正数相加。"""
        assert self.calc.add(3, 5) == 8

    def test_add_positive_and_negative(self):
        """测试正数与负数相加。"""
        assert self.calc.add(10, -3) == 7

    def test_add_two_negative_numbers(self):
        """测试两个负数相加。"""
        assert self.calc.add(-5, -7) == -12

    def test_add_with_zero(self):
        """测试加零。"""
        assert self.calc.add(5, 0) == 5
        assert self.calc.add(0, 5) == 5

    def test_add_float_numbers(self):
        """测试浮点数相加。"""
        result = self.calc.add(3.14, 2.86)
        assert abs(result - 6.0) < 1e-10

    def test_add_large_numbers(self):
        """测试大数相加。"""
        result = self.calc.add(10**12, 10**12)
        assert result == 2 * 10**12

    # ========== 减法测试 ==========

    def test_subtract_positive_numbers(self):
        """测试正数相减。"""
        assert self.calc.subtract(10, 3) == 7

    def test_subtract_result_negative(self):
        """测试结果为负数的情况。"""
        assert self.calc.subtract(3, 10) == -7

    def test_subtract_negative_numbers(self):
        """测试负数相减。"""
        assert self.calc.subtract(-5, -3) == -2

    def test_subtract_with_zero(self):
        """测试减零。"""
        assert self.calc.subtract(5, 0) == 5

    def test_subtract_float_numbers(self):
        """测试浮点数相减。"""
        result = self.calc.subtract(5.5, 1.2)
        assert abs(result - 4.3) < 1e-10

    # ========== 乘法测试 ==========

    def test_multiply_two_positive(self):
        """测试两个正数相乘。"""
        assert self.calc.multiply(4, 5) == 20

    def test_multiply_with_negative(self):
        """测试与负数相乘。"""
        assert self.calc.multiply(6, -2) == -12

    def test_multiply_two_negative(self):
        """测试两个负数相乘。"""
        assert self.calc.multiply(-3, -4) == 12

    def test_multiply_with_zero(self):
        """测试乘以零。"""
        assert self.calc.multiply(5, 0) == 0

    def test_multiply_float_numbers(self):
        """测试浮点数相乘。"""
        result = self.calc.multiply(2.5, 4.0)
        assert abs(result - 10.0) < 1e-10

    # ========== 除法测试 ==========

    def test_divide_exact(self):
        """测试整除。"""
        assert self.calc.divide(10, 2) == 5

    def test_divide_with_remainder(self):
        """测试有余数的除法。"""
        result = self.calc.divide(10, 3)
        assert abs(result - 3.3333333333333335) < 1e-10

    def test_divide_negative_numbers(self):
        """测试负数相除。"""
        assert self.calc.divide(-10, 2) == -5

    def test_divide_float_numbers(self):
        """测试浮点数相除。"""
        result = self.calc.divide(7.0, 2.0)
        assert abs(result - 3.5) < 1e-10

    def test_divide_by_zero_raises_error(self):
        """测试除零异常。"""
        with pytest.raises(CalculatorError, match="除数不能为零"):
            self.calc.divide(5, 0)

    def test_divide_zero_by_number(self):
        """测试零除以非零数。"""
        assert self.calc.divide(0, 5) == 0

    # ========== 幂运算测试 ==========

    def test_power_positive_exponent(self):
        """测试正整数次幂。"""
        assert self.calc.power(2, 3) == 8

    def test_power_zero_exponent(self):
        """测试零次幂。"""
        assert self.calc.power(5, 0) == 1

    def test_power_negative_exponent(self):
        """测试负整数次幂。"""
        result = self.calc.power(2, -2)
        assert abs(result - 0.25) < 1e-10

    def test_power_float_base(self):
        """测试浮点数为底的幂运算。"""
        result = self.calc.power(4.0, 0.5)
        assert abs(result - 2.0) < 1e-10

    # ========== 平方根测试 ==========

    def test_sqrt_perfect_square(self):
        """测试完全平方数的平方根。"""
        assert self.calc.sqrt(9) == 3

    def test_sqrt_non_perfect_square(self):
        """测试非完全平方数的平方根。"""
        result = self.calc.sqrt(2)
        assert abs(result - math.sqrt(2)) < 1e-10

    def test_sqrt_zero(self):
        """测试零的平方根。"""
        assert self.calc.sqrt(0) == 0

    def test_sqrt_negative_raises_error(self):
        """测试负数开平方异常。"""
        with pytest.raises(CalculatorError, match="不能对负数开平方"):
            self.calc.sqrt(-4)

    # ========== 取模测试 ==========

    def test_modulo_positive_numbers(self):
        """测试正数取模。"""
        assert self.calc.modulo(10, 3) == 1

    def test_modulo_exact_division(self):
        """测试整除时取模。"""
        assert self.calc.modulo(10, 5) == 0

    def test_modulo_negative_numbers(self):
        """测试负数取模。"""
        assert self.calc.modulo(-10, 3) == 2

    def test_modulo_by_zero_raises_error(self):
        """测试取模除零异常。"""
        with pytest.raises(CalculatorError, match="取模运算中除数不能为零"):
            self.calc.modulo(5, 0)

    # ========== 输入验证测试 ==========

    def test_invalid_string_input_raises_error(self):
        """测试无效字符串输入异常。"""
        with pytest.raises(CalculatorError, match="无效的输入类型"):
            self.calc.add("a", 5)

    def test_invalid_list_input_raises_error(self):
        """测试无效列表输入异常。"""
        with pytest.raises(CalculatorError, match="无效的输入类型"):
            self.calc.multiply([1, 2], 5)

    def test_invalid_none_input_raises_error(self):
        """测试 None 输入异常。"""
        with pytest.raises(CalculatorError, match="无效的输入类型"):
            self.calc.add(None, 5)

    # ========== 边界测试 ==========

    def test_very_large_numbers(self):
        """测试极大数运算。"""
        large = 10**18
        assert self.calc.add(large, large) == 2 * large

    def test_very_small_float(self):
        """测试极小的浮点数。"""
        small = 1e-15
        result = self.calc.add(small, small)
        assert result == 2e-15

    def test_division_precision(self):
        """测试除法精度。"""
        result = self.calc.divide(1, 3)
        # 1/3 应该接近 0.3333333333333333
        assert abs(result - 1/3) < 1e-15


class TestCalculatorEdgeCases:
    """Calculator 类的边界和特殊场景测试。"""

    def setup_method(self):
        self.calc = Calculator()

    def test_multiple_operations_consistency(self):
        """测试连续运算的一致性。"""
        a = self.calc.add(5, 3)
        b = self.calc.multiply(a, 2)
        c = self.calc.divide(b, 4)
        d = self.calc.subtract(c, 1)
        assert d == 3

    def test_commutative_property_addition(self):
        """测试加法交换律。"""
        assert self.calc.add(3, 7) == self.calc.add(7, 3)

    def test_commutative_property_multiplication(self):
        """测试乘法交换律。"""
        assert self.calc.multiply(4, 5) == self.calc.multiply(5, 4)

    def test_associative_property_addition(self):
        """测试加法结合律。"""
        a = self.calc.add(self.calc.add(2, 3), 5)
        b = self.calc.add(2, self.calc.add(3, 5))
        assert a == b

    def test_zero_division_property(self):
        """测试零除以非零数的属性。"""
        assert self.calc.divide(0, 100) == 0


if __name__ == "__main__":
    pytest.main(["-v", __file__])
