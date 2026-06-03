# Python 计算器程序

一个基于命令行的 Python 计算器程序，支持基本算术运算和数学函数。采用面向对象设计，计算逻辑与用户界面分离，便于测试和扩展。

## 功能特性

- **基本运算**: 加法、减法、乘法、除法
- **高级运算**: 幂运算、平方根、取模
- **友好的命令行界面**: 菜单驱动，操作直观
- **完善的错误处理**: 所有异常情况均有友好提示
- **全面的测试覆盖**: 包含正常场景、边界条件和异常测试

## 快速开始

```bash
cd calculator_project
python calculator.py
```

## 运行测试

```bash
pip install pytest
pytest test_calculator.py -v
```

## 项目结构

```
calculator_project/
├── calculator.py      # 计算器源代码（Calculator 类和 CLI 界面）
├── test_calculator.py # 单元测试文件（pytest）
├── design.md          # 设计文档（含 Mermaid 架构图）
├── user_guide.md      # 用户指南
└── README.md          # 项目说明
```

## 技术栈

- **语言**: Python 3.8+
- **测试框架**: pytest
- **文档**: Markdown + Mermaid
