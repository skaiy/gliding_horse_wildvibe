#!/usr/bin/env python3
"""
计算器程序 - Calculator

一个基于命令行的计算器程序，支持基本算术运算和高级数学函数。
采用面向对象设计，计算逻辑与用户界面分离。
"""

import math


class CalculatorError(Exception):
    """计算器自定义异常类，用于区分计算错误与系统错误。"""
    pass


class Calculator:
    """
    计算引擎类，封装所有数学运算逻辑。

    支持运算：
    - 加法 (add)
    - 减法 (subtract)
    - 乘法 (multiply)
    - 除法 (divide)
    - 幂运算 (power)
    - 平方根 (sqrt)
    - 取模 (modulo)
    """

    @staticmethod
    def _validate_number(value):
        """验证输入是否为有效的数字。"""
        if not isinstance(value, (int, float)):
            raise CalculatorError(f"无效的输入类型: {type(value).__name__}，需要数字类型")
        return True

    def add(self, a, b):
        """加法运算: a + b"""
        self._validate_number(a)
        self._validate_number(b)
        return a + b

    def subtract(self, a, b):
        """减法运算: a - b"""
        self._validate_number(a)
        self._validate_number(b)
        return a - b

    def multiply(self, a, b):
        """乘法运算: a * b"""
        self._validate_number(a)
        self._validate_number(b)
        return a * b

    def divide(self, a, b):
        """除法运算: a / b

        当除数为零时抛出 CalculatorError。
        """
        self._validate_number(a)
        self._validate_number(b)
        if b == 0:
            raise CalculatorError("除数不能为零")
        return a / b

    def power(self, a, b):
        """幂运算: a ** b"""
        self._validate_number(a)
        self._validate_number(b)
        return a ** b

    def sqrt(self, a):
        """平方根运算: sqrt(a)

        当输入为负数时抛出 CalculatorError。
        """
        self._validate_number(a)
        if a < 0:
            raise CalculatorError("不能对负数开平方")
        return math.sqrt(a)

    def modulo(self, a, b):
        """取模运算: a % b"""
        self._validate_number(a)
        self._validate_number(b)
        if b == 0:
            raise CalculatorError("取模运算中除数不能为零")
        return a % b


class CalculatorCLI:
    """命令行界面类，负责与用户的输入输出交互。"""

    # 菜单选项定义
    MENU_ITEMS = [
        ("1", "加法", "add", 2),
        ("2", "减法", "subtract", 2),
        ("3", "乘法", "multiply", 2),
        ("4", "除法", "divide", 2),
        ("5", "幂运算", "power", 2),
        ("6", "平方根", "sqrt", 1),
        ("7", "取模", "modulo", 2),
        ("0", "退出", None, 0),
    ]

    def __init__(self):
        self._calculator = Calculator()

    def run(self):
        """启动计算器主循环。"""
        self._show_welcome()
        while True:
            self._show_menu()
            choice = input("\n请选择操作 (0-7): ").strip()
            if choice == "0":
                print("\n感谢使用计算器，再见！")
                break
            self._handle_choice(choice)

    def _show_welcome(self):
        """显示欢迎信息。"""
        print("=" * 50)
        print("         欢迎使用 Python 计算器")
        print("=" * 50)

    def _show_menu(self):
        """显示功能菜单。"""
        print("\n--- 功能菜单 ---")
        for key, label, _, _ in self.MENU_ITEMS:
            print(f"  {key}. {label}")

    def _get_number(self, prompt):
        """获取用户输入的数字，包含错误处理。"""
        while True:
            try:
                value = input(prompt).strip()
                # 支持整数和浮点数输入
                if "." in value:
                    return float(value)
                return int(value)
            except ValueError:
                print("输入错误：请输入有效的数字！")

    def _handle_choice(self, choice):
        """处理用户选择的操作。"""
        # 查找匹配的菜单项
        menu_item = None
        for key, label, method, num_args in self.MENU_ITEMS:
            if key == choice:
                menu_item = (key, label, method, num_args)
                break

        if menu_item is None:
            print("无效选择，请重新输入！")
            return

        _, label, method_name, num_args = menu_item
        print(f"\n--- {label} ---")

        try:
            # 获取操作数
            if num_args == 1:
                a = self._get_number("请输入数字: ")
                result = getattr(self._calculator, method_name)(a)
            elif num_args == 2:
                a = self._get_number("请输入第一个数字: ")
                b = self._get_number("请输入第二个数字: ")
                result = getattr(self._calculator, method_name)(a, b)
            else:
                return

            # 格式化结果
            result_str = self._format_result(result)
            print(f"\n✅ 结果: {result_str}")

        except CalculatorError as e:
            print(f"\n❌ 计算错误: {e}")
        except Exception as e:
            print(f"\n❌ 未知错误: {e}")

    @staticmethod
    def _format_result(value):
        """格式化计算结果，如果是整数则不显示小数部分。"""
        if isinstance(value, float) and value == int(value):
            return str(int(value))
        return str(value)


def main():
    """程序入口函数。"""
    cli = CalculatorCLI()
    cli.run()


if __name__ == "__main__":
    main()
