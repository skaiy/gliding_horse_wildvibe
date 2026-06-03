# 计算器程序 - 设计文档

## 1. 概述

本文档描述 Python 计算器程序的设计思路、架构和核心流程。该计算器支持基本的四则运算（加、减、乘、除）以及辅助功能。

## 2. 系统架构

计算器采用**分层架构**，共分为三层：

```
┌──────────────────────┐
│    用户界面层 (UI)    │  ← 命令行交互 / 输入输出
├──────────────────────┤
│     业务逻辑层        │  ← 运算调度、表达式解析
├──────────────────────┤
│     核心计算层        │  ← 四则运算实现
└──────────────────────┘
```

### 2.1 模块职责

| 模块 | 职责 |
|------|------|
| `calculator.py` | 主入口，命令行交互，用户输入处理 |
| `core.py` | 核心运算函数（加、减、乘、除） |
| `operations.py` | 运算调度与校验 |

## 3. 功能需求

- 支持加法 (`+`)
- 支持减法 (`-`)
- 支持乘法 (`*`)
- 支持除法 (`/`)，包含除零错误处理
- 支持连续运算（基于上一次结果）
- 清空重置功能
- 友好的错误提示

## 4. 核心流程图

```mermaid
flowchart TD
    Start([启动程序]) --> ShowMenu[显示操作菜单]
    ShowMenu --> Input[用户输入表达式<br/>如: 3 + 5]
    Input --> Parse{解析输入}
    
    Parse -->|格式正确| Validate{验证操作数}
    Parse -->|格式错误| Error1[提示: 输入格式有误]
    Error1 --> ShowMenu
    
    Validate -->|合法数字| CheckOp{检查运算符}
    Validate -->|非法数字| Error2[提示: 请输入有效数字]
    Error2 --> ShowMenu
    
    CheckOp -->|加法| Add[计算 a + b]
    CheckOp -->|减法| Sub[计算 a - b]
    CheckOp -->|乘法| Mul[计算 a * b]
    CheckOp -->|除法| Div[计算 a / b]
    CheckOp -->|未知运算符| Error3[提示: 不支持的运算符]
    Error3 --> ShowMenu
    
    Div --> CheckZero{b == 0?}
    CheckZero -->|是| Error4[提示: 除数不能为零]
    CheckZero -->|否| CalcDiv[返回 a / b]
    
    Add --> Result[显示结果]
    Sub --> Result
    Mul --> Result
    CalcDiv --> Result
    Error4 --> ShowMenu
    
    Result --> Continue{继续运算?}
    Continue -->|是| ShowMenu
    Continue -->|否| End([程序结束])
```

## 5. 类设计

```mermaid
classDiagram
    class Calculator {
        +run() void
        -display_menu() void
        -get_input() str
        -process_expression(expr) float
    }
    
    class CoreOperations {
        +add(a, b) float
        +subtract(a, b) float
        +multiply(a, b) float
        +divide(a, b) float
    }
    
    class InputParser {
        +parse(expr) tuple
        -validate_number(s) bool
    }
    
    Calculator --> CoreOperations : 调用
    Calculator --> InputParser : 使用
    
    CoreOperations ..> ZeroDivisionError : 抛出
```

## 6. 数据流

```mermaid
sequenceDiagram
    participant User as 用户
    participant UI as Calculator(UI)
    participant Parser as InputParser
    participant Core as CoreOperations
    
    User->>UI: 输入 "3 + 5"
    UI->>Parser: parse("3 + 5")
    Parser->>Parser: 拆分并验证
    Parser-->>UI: (3.0, '+', 5.0)
    UI->>Core: add(3.0, 5.0)
    Core-->>UI: 8.0
    UI-->>User: 显示结果: 8.0
```

## 7. 错误处理策略

| 错误场景 | 处理方式 |
|----------|----------|
| 输入格式错误 | 提示用户输入格式应为 "a + b" |
| 非数字输入 | 捕获 ValueError，提示输入有效数字 |
| 除零操作 | 捕获 ZeroDivisionError，提示除数不能为零 |
| 不支持的运算符 | 提示支持的运算符列表 |

## 8. 测试策略

- **单元测试**: 对 `CoreOperations` 每个方法进行独立测试
- **异常测试**: 验证除零、非法输入等场景
- **集成测试**: 验证完整运算流程

---

*版本: 1.0*
*最后更新: 2024年*
